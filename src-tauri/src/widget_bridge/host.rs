use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::time::UNIX_EPOCH;

use super::model::WidgetSnapshot;
#[cfg(target_os = "macos")]
use super::HOST_APP_NAME;
use super::{DEV_WIDGET_BUNDLE_ID, SNAPSHOT_NAME, WIDGET_BUNDLE_ID};

pub(super) fn write_snapshot(snapshot: &WidgetSnapshot) -> Result<(), String> {
    let json = serde_json::to_string_pretty(snapshot).map_err(|error| error.to_string())?;
    let mut wrote = 0usize;
    let mut optional_errors = Vec::new();
    for path in snapshot_paths() {
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                if optional_snapshot_path(&path) {
                    optional_errors.push(format!("{}: {error}", path.display()));
                    continue;
                }
                return Err(error.to_string());
            }
        }
        match write_atomic(&path, json.as_bytes()) {
            Ok(()) => wrote += 1,
            Err(error) if optional_snapshot_path(&path) => {
                optional_errors.push(format!("{}: {error}", path.display()));
            }
            Err(error) => return Err(error),
        }
    }
    if wrote == 0 {
        return Err(if optional_errors.is_empty() {
            "widget snapshot path is unavailable".to_string()
        } else {
            optional_errors.join("; ")
        });
    }
    if !optional_errors.is_empty() {
        static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            log::warn!(
                "widget snapshot skipped an optional container path: {}",
                optional_errors.join("; ")
            );
        }
    }
    Ok(())
}

fn optional_snapshot_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains("Group Containers") || text.contains("/Containers/")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp).map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().ok();
    }
    fs::rename(&tmp, path).map_err(|error| error.to_string())
}

fn snapshot_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(data) = dirs::data_dir() {
        paths.push(data.join("com.kgu.selah").join(SNAPSHOT_NAME));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(
            home.join("Library/Group Containers/group.com.kgu.selah")
                .join(SNAPSHOT_NAME),
        );
        paths.push(
            home.join("Library/Containers")
                .join(WIDGET_BUNDLE_ID)
                .join("Data/Library/Application Support/com.kgu.selah")
                .join(SNAPSHOT_NAME),
        );
    }
    paths
}

pub(super) fn reload_timelines() {
    #[cfg(target_os = "macos")]
    {
        if let Some(api) = widget_api() {
            unsafe { (api.reload)() };
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn running_inside_widget_app() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(parent) = exe.parent() else {
        return false;
    };
    parent.join("../PlugIns/SelahWidget.appex").exists()
}

#[cfg(target_os = "macos")]
pub(super) fn install_host() -> Result<(), String> {
    let appex = PathBuf::from(env!("SELAH_WIDGET_APPEX"));
    let host_bin = PathBuf::from(env!("SELAH_WIDGET_HOST"));
    if !appex.is_dir() || !host_bin.is_file() {
        return Err("widget bundle was not built".into());
    }
    let dest = applications_dir()?.join(HOST_APP_NAME);
    let stamp = dest.join("Contents/Resources/widget-stamp");
    let source_stamp = stamp_value(&appex.join("Contents/MacOS/SelahWidget"))?;
    if stamp.is_file()
        && fs::read_to_string(&stamp).ok().as_deref() == Some(source_stamp.as_str())
        && host_extension_is_retargeted(&dest)
    {
        launch_host(&dest);
        return Ok(());
    }
    if dest.exists() {
        let _ = Command::new("pkill")
            .args(["-f", "selah-widget-host"])
            .status();
        let mut last_error = String::from("widget host is still running");
        for _ in 0..20 {
            match fs::remove_dir_all(&dest) {
                Ok(()) => {
                    last_error.clear();
                    break;
                }
                Err(error) => {
                    last_error = error.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
        if dest.exists() {
            return Err(last_error);
        }
    }
    let plugins = dest.join("Contents/PlugIns");
    let macos = dest.join("Contents/MacOS");
    let resources = dest.join("Contents/Resources");
    fs::create_dir_all(&plugins).map_err(|error| error.to_string())?;
    fs::create_dir_all(&macos).map_err(|error| error.to_string())?;
    fs::create_dir_all(&resources).map_err(|error| error.to_string())?;
    let host_appex = plugins.join("SelahWidget.appex");
    copy_dir(&appex, &host_appex)?;
    retarget_host_extension(&host_appex)?;
    fs::copy(&host_bin, macos.join("selah-widget-host")).map_err(|error| error.to_string())?;
    let icon = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("icons/icon.icns");
    if icon.is_file() {
        let _ = fs::copy(icon, resources.join("icon.icns"));
    }
    fs::write(dest.join("Contents/Info.plist"), host_info_plist())
        .map_err(|error| error.to_string())?;
    fs::write(&stamp, &source_stamp).map_err(|error| error.to_string())?;
    sign_host(&dest)?;
    register_host(&dest)?;
    launch_host(&dest);
    log::info!("registered macOS widget host at {}", dest.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn applications_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("home directory is unavailable")?;
    let dir = home.join("Applications");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

#[cfg(target_os = "macos")]
fn stamp_value(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|error| error.to_string())?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|time| time.as_secs())
        .unwrap_or(0);
    Ok(format!("{}:{}", meta.len(), modified))
}

#[cfg(target_os = "macos")]
fn host_info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>ja</string>
    <key>CFBundleExecutable</key><string>selah-widget-host</string>
    <key>CFBundleIdentifier</key><string>com.kgu.selah.widget-host</string>
    <key>CFBundleName</key><string>Selah</string>
    <key>CFBundleDisplayName</key><string>Selah</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>1.1.0</string>
    <key>CFBundleVersion</key><string>1.1.0</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSUIElement</key><true/>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "macos")]
fn sign_host(app: &Path) -> Result<(), String> {
    let entitlements =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("swift/widget/Widget.entitlements");
    let appex = app.join("Contents/PlugIns/SelahWidget.appex");
    run_status(
        "codesign",
        &[
            "--force",
            "--sign",
            "-",
            "--entitlements",
            &entitlements.to_string_lossy(),
            &appex.to_string_lossy(),
        ],
    )?;
    run_status(
        "codesign",
        &["--force", "--sign", "-", &app.to_string_lossy()],
    )
}

#[cfg(target_os = "macos")]
pub(super) fn retire_dev_host() -> Result<(), String> {
    unregister_foreign_widget_copies();
    let dest = applications_dir()?.join(HOST_APP_NAME);
    let appex = dest.join("Contents/PlugIns/SelahWidget.appex");
    if appex.exists() {
        let _ = Command::new("pluginkit")
            .args(["-r", &appex.to_string_lossy()])
            .status();
    }
    if host_is_running() {
        let _ = Command::new("pkill")
            .args(["-f", "selah-widget-host"])
            .status();
    }
    if dest.exists() {
        let mut last_error = String::from("widget dev host is still running");
        for _ in 0..20 {
            match fs::remove_dir_all(&dest) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    last_error = error.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
        if dest.exists() {
            return Err(last_error);
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn retarget_host_extension(appex: &Path) -> Result<(), String> {
    let plist = appex.join("Contents/Info.plist");
    let text = fs::read_to_string(&plist).map_err(|error| error.to_string())?;
    fs::write(&plist, host_extension_plist(&text)).map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn host_extension_is_retargeted(app: &Path) -> bool {
    let plist = app.join("Contents/PlugIns/SelahWidget.appex/Contents/Info.plist");
    fs::read_to_string(plist)
        .map(|text| text.contains(DEV_WIDGET_BUNDLE_ID))
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn unregister_foreign_widget_copies() {
    let own = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|parent| parent.join("../PlugIns/SelahWidget.appex"))
        })
        .and_then(|path| path.canonicalize().ok());
    let Ok(output) = Command::new("pluginkit")
        .args(["-m", "-A", "-v", "-i", WIDGET_BUNDLE_ID])
        .output()
    else {
        return;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let Some(path) = line.split('\t').last().map(str::trim) else {
            continue;
        };
        if !path.ends_with("SelahWidget.appex") {
            continue;
        }
        let candidate = Path::new(path);
        if own.as_ref().is_some_and(|own| paths_match(candidate, own)) {
            continue;
        }
        let _ = Command::new("pluginkit").args(["-r", path]).status();
    }
}

#[cfg(target_os = "macos")]
fn paths_match(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

pub(super) fn host_extension_plist(plist: &str) -> String {
    plist.replace(
        &format!("<string>{WIDGET_BUNDLE_ID}</string>"),
        &format!("<string>{DEV_WIDGET_BUNDLE_ID}</string>"),
    )
}

#[cfg(target_os = "macos")]
fn register_host(app: &Path) -> Result<(), String> {
    let appex = app.join("Contents/PlugIns/SelahWidget.appex");
    let _ = Command::new("pluginkit")
        .args(["-a", &appex.to_string_lossy()])
        .status();
    let _ = Command::new("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister")
        .args(["-f", &app.to_string_lossy()])
        .status();
    let _ = WIDGET_BUNDLE_ID;
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_host(app: &Path) {
    if host_is_running() {
        return;
    }
    let _ = Command::new("open")
        .args(["-g", "-j", &app.to_string_lossy()])
        .status();
}

#[cfg(target_os = "macos")]
fn host_is_running() -> bool {
    Command::new("pgrep")
        .args(["-f", "selah-widget-host"])
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(dest).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(src).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = dest.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_status(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("failed to run {program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed"))
    }
}

pub(super) fn macos_major() -> u32 {
    crate::local_ai_support::macos_major_version().unwrap_or(0)
}

#[cfg(target_os = "macos")]
type ReloadFn = unsafe extern "C" fn();

#[derive(Clone, Copy)]
#[cfg(target_os = "macos")]
struct WidgetApi {
    reload: ReloadFn,
}

#[cfg(target_os = "macos")]
fn widget_api() -> Option<WidgetApi> {
    static LOADED: std::sync::OnceLock<Option<WidgetApi>> = std::sync::OnceLock::new();
    LOADED.get_or_init(load_widget_api).clone()
}

#[cfg(target_os = "macos")]
fn load_widget_api() -> Option<WidgetApi> {
    let path = widget_library_path()?;
    let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let handle = unsafe { dlopen(c_path.as_ptr(), 2 | 4) };
    if handle.is_null() {
        return None;
    }
    let name = std::ffi::CString::new("selah_widget_reload").ok()?;
    let symbol = unsafe { dlsym(handle, name.as_ptr()) };
    if symbol.is_null() {
        return None;
    }
    Some(WidgetApi {
        reload: unsafe { std::mem::transmute_copy(&symbol) },
    })
}

#[cfg(target_os = "macos")]
fn widget_library_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("SELAH_WIDGET_LIB") {
        candidates.push(PathBuf::from(path));
    }
    candidates.push(PathBuf::from(env!("SELAH_WIDGET_LIB")));
    candidates.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/lib/libselah_widget.dylib"
    )));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("../Frameworks/libselah_widget.dylib"));
            candidates.push(parent.join("libselah_widget.dylib"));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(target_os = "macos")]
extern "C" {
    fn dlopen(path: *const std::os::raw::c_char, flags: i32) -> *mut std::ffi::c_void;
    fn dlsym(
        handle: *mut std::ffi::c_void,
        symbol: *const std::os::raw::c_char,
    ) -> *mut std::ffi::c_void;
}
