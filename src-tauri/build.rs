use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    #[cfg(target_os = "windows")]
    {
        embed_resource::compile_for_everything("windows-tests.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to compile the Windows application manifest");

        let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
            .expect("failed to build the Tauri Windows resources");
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(error) = compile_apple_intelligence_bridge() {
            panic!("failed to build the Apple Intelligence bridge: {error}");
        }
        if let Err(error) = compile_macos_widget() {
            panic!("failed to build the macOS widget: {error}");
        }
    }

    #[cfg(not(target_os = "windows"))]
    tauri_build::build();
}

#[cfg(target_os = "macos")]
fn compile_apple_intelligence_bridge() -> Result<(), String> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest_dir.join("swift/AppleIntelligenceBridge.swift");
    println!("cargo:rerun-if-changed={}", source.display());
    let bundled = manifest_dir.join("lib/libselah_apple_ai.dylib");
    if !output_is_stale(&bundled, &[&source]) {
        println!("cargo:rustc-env=SELAH_APPLE_AI_LIB={}", bundled.display());
        return Ok(());
    }

    let sdk = command_stdout("xcrun", &["--show-sdk-path"])?;
    let framework =
        PathBuf::from(sdk.trim()).join("System/Library/Frameworks/FoundationModels.framework");
    if !framework.exists() {
        return Err(format!(
            "FoundationModels.framework was not found in {} . Install the macOS 26 SDK or newer.",
            framework.display()
        ));
    }
    let swiftc = command_stdout("xcrun", &["--find", "swiftc"])?;
    let swiftc = swiftc.trim();
    if swiftc.is_empty() {
        return Err("swiftc was not found. Install the Xcode Command Line Tools.".into());
    }
    let host_target = swift_target(&host_arch(), "26.0").unwrap_or_default();
    let fm27 = foundation_models_27(swiftc, sdk.trim(), &host_target);
    if !fm27 {
        println!(
            "cargo:warning=FoundationModels SDK has no macOS 27 symbols; building the macOS 26 Apple Intelligence bridge"
        );
    }

    let slices_dir = manifest_dir.join("target/apple-ai-slices");
    std::fs::create_dir_all(&slices_dir).map_err(|error| error.to_string())?;

    let mut built = Vec::new();
    for arch in target_arches() {
        let Some(target) = swift_target(&arch, "26.0") else {
            continue;
        };
        let slice = slices_dir.join(format!("libselah_apple_ai_{arch}.dylib"));
        let mut command = Command::new(swiftc);
        command.args([
            "-emit-library",
            "-parse-as-library",
            "-module-name",
            "SelahAppleAI",
            "-target",
            &target,
            "-sdk",
            sdk.trim(),
            "-O",
            "-framework",
            "Foundation",
            "-Xlinker",
            "-weak_framework",
            "-Xlinker",
            "FoundationModels",
            "-Xlinker",
            "-rpath",
            "-Xlinker",
            "/usr/lib/swift",
            "-Xlinker",
            "-install_name",
            "-Xlinker",
            "@rpath/libselah_apple_ai.dylib",
        ]);
        if fm27 {
            command.args(["-D", "SELAH_FM_27"]);
        }
        let status = command
            .args(["-o"])
            .arg(&slice)
            .arg(&source)
            .status()
            .map_err(|error| format!("failed to run swiftc: {error}"))?;
        if status.success() {
            built.push(slice);
            continue;
        }
        if arch == host_arch() {
            return Err(format!("swiftc failed while building {target}"));
        }
        println!(
            "cargo:warning=Apple Intelligence bridge skipped {target}; the bundled library will not include this architecture"
        );
    }

    if built.is_empty() {
        return Err("no Apple Intelligence bridge was built".into());
    }

    lipo(&built, &bundled, "libselah_apple_ai.dylib")?;
    let _ = Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&bundled)
        .status();
    println!("cargo:rustc-env=SELAH_APPLE_AI_LIB={}", bundled.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn foundation_models_27(swiftc: &str, sdk: &str, target: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    let dir = std::env::temp_dir().join(format!("selah-fm-probe-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let source = dir.join("probe.swift");
    let snippet = "import FoundationModels\n@available(macOS 27.0, *)\nfunc selahProbe(_ error: LanguageModelError) {}\n";
    if std::fs::write(&source, snippet).is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        return false;
    }
    let ok = Command::new(swiftc)
        .args([
            "-parse-as-library",
            "-typecheck",
            "-target",
            target,
            "-sdk",
            sdk,
            "-framework",
            "Foundation",
        ])
        .arg(&source)
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

#[cfg(target_os = "macos")]
fn compile_macos_widget() -> Result<(), String> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let widget_src = manifest_dir.join("swift/widget/SelahWidget.swift");
    let host_src = manifest_dir.join("swift/widget/WidgetHost.swift");
    let bridge_src = manifest_dir.join("swift/WidgetBridge.swift");
    let entitlements = widget_entitlements(&manifest_dir);
    let app_store_entitlements = manifest_dir.join("swift/widget/Widget.entitlements");
    let developer_id_entitlements =
        manifest_dir.join("swift/widget/Widget.DeveloperID.entitlements");
    let logo = manifest_dir.join("../src/assets/logo.png");
    for path in [
        &widget_src,
        &host_src,
        &bridge_src,
        &entitlements,
        &app_store_entitlements,
        &developer_id_entitlements,
        &logo,
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let lib_dir = manifest_dir.join("lib");
    let host_bin = lib_dir.join("selah-widget-host");
    let bridge_bin = lib_dir.join("libselah_widget.dylib");
    let appex = lib_dir.join("SelahWidget.appex");
    let executable = appex.join("Contents/MacOS/SelahWidget");
    let inputs = [
        widget_src.as_path(),
        host_src.as_path(),
        bridge_src.as_path(),
        entitlements.as_path(),
        logo.as_path(),
    ];
    let bundled_logo = appex.join("Contents/Resources/logo.png");
    if !output_is_stale(&executable, &inputs)
        && !output_is_stale(&host_bin, &inputs)
        && !output_is_stale(&bridge_bin, &inputs)
        && appex.join("Contents/Info.plist").is_file()
        && bundled_logo.is_file()
    {
        println!("cargo:rustc-env=SELAH_WIDGET_APPEX={}", appex.display());
        println!("cargo:rustc-env=SELAH_WIDGET_HOST={}", host_bin.display());
        println!("cargo:rustc-env=SELAH_WIDGET_LIB={}", bridge_bin.display());
        return Ok(());
    }

    let sdk = command_stdout("xcrun", &["--show-sdk-path"])?;
    let swiftc = command_stdout("xcrun", &["--find", "swiftc"])?;
    let swiftc = swiftc.trim();
    if swiftc.is_empty() {
        return Err("swiftc was not found. Install the Xcode Command Line Tools.".into());
    }

    let slices_dir = manifest_dir.join("target/widget-slices");
    std::fs::create_dir_all(&slices_dir).map_err(|error| error.to_string())?;

    let mut widget_slices = Vec::new();
    let mut host_slices = Vec::new();
    let mut bridge_slices = Vec::new();
    for arch in target_arches() {
        let Some(target) = swift_target(&arch, "14.0") else {
            continue;
        };
        let widget_slice = slices_dir.join(format!("SelahWidget_{arch}"));
        let host_slice = slices_dir.join(format!("selah-widget-host_{arch}"));
        let bridge_slice = slices_dir.join(format!("libselah_widget_{arch}.dylib"));
        if let Err(error) =
            compile_widget_slice(swiftc, sdk.trim(), &target, &widget_src, &widget_slice)
        {
            if arch == host_arch() {
                return Err(error);
            }
            println!("cargo:warning=macOS widget skipped {target}: {error}");
            continue;
        }
        assert_extension_entry(&widget_slice)?;
        compile_host_slice(swiftc, sdk.trim(), &target, &host_src, &host_slice)?;
        compile_bridge_slice(swiftc, sdk.trim(), &target, &bridge_src, &bridge_slice)?;
        widget_slices.push(widget_slice);
        host_slices.push(host_slice);
        bridge_slices.push(bridge_slice);
    }

    if widget_slices.is_empty() {
        return Err("no macOS widget was built".into());
    }

    let widget_bin = slices_dir.join("SelahWidget");
    lipo(&widget_slices, &widget_bin, "SelahWidget")?;
    lipo(&host_slices, &host_bin, "selah-widget-host")?;
    lipo(&bridge_slices, &bridge_bin, "libselah_widget.dylib")?;
    make_executable(&widget_bin)?;
    make_executable(&host_bin)?;
    sign_adhoc(&host_bin)?;
    sign_adhoc(&bridge_bin)?;

    if appex.exists() {
        std::fs::remove_dir_all(&appex).map_err(|error| error.to_string())?;
    }
    let macos = appex.join("Contents/MacOS");
    std::fs::create_dir_all(&macos).map_err(|error| error.to_string())?;
    let executable = macos.join("SelahWidget");
    std::fs::copy(&widget_bin, &executable).map_err(|error| error.to_string())?;
    make_executable(&executable)?;
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "1.1.0".into());
    std::fs::write(
        appex.join("Contents/Info.plist"),
        widget_info_plist(&version),
    )
    .map_err(|error| error.to_string())?;
    let resources = appex.join("Contents/Resources");
    std::fs::create_dir_all(&resources).map_err(|error| error.to_string())?;
    if !logo.is_file() {
        return Err(format!("widget logo was not found at {}", logo.display()));
    }
    std::fs::copy(&logo, resources.join("logo.png")).map_err(|error| error.to_string())?;
    sign_appex(&appex, &entitlements)?;

    println!("cargo:rustc-env=SELAH_WIDGET_APPEX={}", appex.display());
    println!("cargo:rustc-env=SELAH_WIDGET_HOST={}", host_bin.display());
    println!("cargo:rustc-env=SELAH_WIDGET_LIB={}", bridge_bin.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn compile_widget_slice(
    swiftc: &str,
    sdk: &str,
    target: &str,
    source: &Path,
    output: &Path,
) -> Result<(), String> {
    swiftc_output(
        swiftc,
        &[
            "-emit-executable",
            "-parse-as-library",
            "-module-name",
            "SelahWidget",
            "-application-extension",
            "-target",
            target,
            "-sdk",
            sdk,
            "-O",
            "-framework",
            "Foundation",
            "-framework",
            "AppKit",
            "-framework",
            "SwiftUI",
            "-framework",
            "WidgetKit",
            "-Xlinker",
            "-e",
            "-Xlinker",
            "_NSExtensionMain",
            "-Xlinker",
            "-rpath",
            "-Xlinker",
            "/usr/lib/swift",
            "-o",
        ],
        output,
        source,
    )
}

#[cfg(target_os = "macos")]
fn compile_host_slice(
    swiftc: &str,
    sdk: &str,
    target: &str,
    source: &Path,
    output: &Path,
) -> Result<(), String> {
    swiftc_output(
        swiftc,
        &[
            "-emit-executable",
            "-module-name",
            "SelahWidgetHost",
            "-target",
            target,
            "-sdk",
            sdk,
            "-O",
            "-framework",
            "Foundation",
            "-framework",
            "AppKit",
            "-framework",
            "WidgetKit",
            "-Xlinker",
            "-rpath",
            "-Xlinker",
            "/usr/lib/swift",
            "-o",
        ],
        output,
        source,
    )
}

#[cfg(target_os = "macos")]
fn compile_bridge_slice(
    swiftc: &str,
    sdk: &str,
    target: &str,
    source: &Path,
    output: &Path,
) -> Result<(), String> {
    swiftc_output(
        swiftc,
        &[
            "-emit-library",
            "-parse-as-library",
            "-module-name",
            "SelahWidgetBridge",
            "-target",
            target,
            "-sdk",
            sdk,
            "-O",
            "-framework",
            "Foundation",
            "-framework",
            "WidgetKit",
            "-Xlinker",
            "-rpath",
            "-Xlinker",
            "/usr/lib/swift",
            "-Xlinker",
            "-install_name",
            "-Xlinker",
            "@rpath/libselah_widget.dylib",
            "-o",
        ],
        output,
        source,
    )
}

#[cfg(target_os = "macos")]
fn swiftc_output(
    swiftc: &str,
    prefix: &[&str],
    output: &Path,
    source: &Path,
) -> Result<(), String> {
    let mut command = Command::new(swiftc);
    command.args(prefix).arg(output).arg(source);
    let result = command
        .output()
        .map_err(|error| format!("failed to run swiftc: {error}"))?;
    if result.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&result.stderr);
    Err(format!(
        "swiftc failed while building {}: {stderr}",
        output.display()
    ))
}

#[cfg(target_os = "macos")]
fn assert_extension_entry(binary: &Path) -> Result<(), String> {
    let output = Command::new("nm")
        .args(["-u"])
        .arg(binary)
        .output()
        .map_err(|error| format!("failed to run nm: {error}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    if text.contains("_NSExtensionMain") {
        Ok(())
    } else {
        Err(format!(
            "{} is missing the _NSExtensionMain entry point",
            binary.display()
        ))
    }
}

#[cfg(target_os = "macos")]
fn widget_info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>ja</string>
    <key>CFBundleDisplayName</key><string>授業とTODO</string>
    <key>CFBundleExecutable</key><string>SelahWidget</string>
    <key>CFBundleIdentifier</key><string>com.kgu.selah.widget</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>SelahWidget</string>
    <key>CFBundlePackageType</key><string>XPC!</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSExtension</key>
    <dict>
        <key>NSExtensionPointIdentifier</key>
        <string>com.apple.widgetkit-extension</string>
        <key>NSExtensionPrincipalClass</key>
        <string>SelahWidget.SelahWidgetBundle</string>
    </dict>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "macos")]
fn widget_entitlements(manifest_dir: &Path) -> PathBuf {
    let identity = std::env::var("APPLE_SIGNING_IDENTITY").unwrap_or_default();
    if identity.contains("Developer ID") {
        let developer_id = manifest_dir.join("swift/widget/Widget.DeveloperID.entitlements");
        if developer_id.is_file() {
            return developer_id;
        }
    }
    manifest_dir.join("swift/widget/Widget.entitlements")
}

#[cfg(target_os = "macos")]
fn sign_appex(appex: &Path, entitlements: &Path) -> Result<(), String> {
    let identity = std::env::var("APPLE_SIGNING_IDENTITY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".to_string());
    let mut command = Command::new("codesign");
    command.args(["--force", "--sign", &identity]);
    if let Ok(keychain) = std::env::var("SELAH_CODESIGN_KEYCHAIN") {
        let keychain = keychain.trim();
        if !keychain.is_empty() {
            command.args(["--keychain", keychain]);
        }
    }
    if identity != "-" {
        command.args(["--options", "runtime", "--timestamp"]);
    }
    command.arg("--entitlements").arg(entitlements).arg(appex);
    let output = command
        .output()
        .map_err(|error| format!("failed to run codesign: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "codesign {} failed: {}",
        appex.display(),
        String::from_utf8_lossy(&output.stderr)
    ))
}

#[cfg(target_os = "macos")]
fn sign_adhoc(path: &Path) -> Result<(), String> {
    let status = Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(path)
        .status()
        .map_err(|error| format!("failed to run codesign: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("codesign {} failed", path.display()))
    }
}

#[cfg(target_os = "macos")]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn output_is_stale(output: &Path, inputs: &[&Path]) -> bool {
    let Ok(output_time) = std::fs::metadata(output).and_then(|meta| meta.modified()) else {
        return true;
    };
    let build_script = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("build.rs");
    inputs
        .iter()
        .copied()
        .chain(std::iter::once(build_script.as_path()))
        .any(|input| {
            std::fs::metadata(input)
                .and_then(|meta| meta.modified())
                .map(|time| time > output_time)
                .unwrap_or(true)
        })
}

#[cfg(target_os = "macos")]
fn host_arch() -> String {
    std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".into())
}

#[cfg(target_os = "macos")]
fn target_arches() -> Vec<String> {
    let host = host_arch();
    let mut arches = vec![host.clone()];
    for arch in ["aarch64", "x86_64"] {
        if !arches.iter().any(|item| item == arch) {
            arches.push(arch.to_string());
        }
    }
    arches
}

#[cfg(target_os = "macos")]
fn swift_target(arch: &str, minimum: &str) -> Option<String> {
    let arch = match arch {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        _ => return None,
    };
    Some(format!("{arch}-apple-macos{minimum}"))
}

#[cfg(target_os = "macos")]
fn lipo(slices: &[PathBuf], output: &Path, label: &str) -> Result<(), String> {
    if slices.len() == 1 {
        std::fs::copy(&slices[0], output).map_err(|error| error.to_string())?;
        return Ok(());
    }
    let mut command = Command::new("lipo");
    command.arg("-create");
    for slice in slices {
        command.arg(slice);
    }
    let status = command
        .arg("-output")
        .arg(output)
        .status()
        .map_err(|error| format!("failed to run lipo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("lipo failed while creating {label}"))
    }
}

#[cfg(target_os = "macos")]
fn command_stdout(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("failed to run {program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
