use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const DIRECTML_RUNTIME_FILES: &[&str] =
    &["sherpa-onnx-c-api.dll", "onnxruntime.dll", "DirectML.dll"];

fn main() {
    println!("cargo:rerun-if-env-changed=SELAH_ENABLE_STT_DIRECTML");
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_LIB_DIR");
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_ARCHIVE_DIR");

    let runtime_dir = PathBuf::from("windows-runtime");
    let _ = fs::create_dir_all(&runtime_dir);

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let directml_requested = env::var("SELAH_ENABLE_STT_DIRECTML")
        .map(|value| {
            let value = value.trim();
            value == "1" || value.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false);

    if target_os == "windows" {
        if directml_requested {
            match env::var_os("SHERPA_ONNX_LIB_DIR") {
                Some(lib_dir) => {
                    let lib_dir = PathBuf::from(lib_dir);
                    println!("cargo:rerun-if-changed={}", lib_dir.display());
                    match stage_directml_runtime(&lib_dir, &runtime_dir) {
                        Ok(()) => {
                            println!("cargo:rustc-env=SELAH_STT_DIRECTML_ENABLED=1");
                        }
                        Err(err) => {
                            println!("cargo:warning={err}");
                        }
                    }
                }
                None => {
                    clear_staged_runtime_dlls(&runtime_dir);
                    if env::var_os("SHERPA_ONNX_ARCHIVE_DIR").is_some() {
                        println!(
                            "cargo:warning=SELAH_ENABLE_STT_DIRECTML is set, but SHERPA_ONNX_LIB_DIR is required so the DirectML runtime DLLs can be staged for packaging"
                        );
                    } else {
                        println!(
                            "cargo:warning=SELAH_ENABLE_STT_DIRECTML is set, but SHERPA_ONNX_LIB_DIR was not provided"
                        );
                    }
                }
            }
        } else {
            clear_staged_runtime_dlls(&runtime_dir);
        }
    }

    tauri_build::build()
}

fn stage_directml_runtime(lib_dir: &Path, runtime_dir: &Path) -> Result<(), String> {
    if !lib_dir.is_dir() {
        return Err(format!(
            "SHERPA_ONNX_LIB_DIR does not exist or is not a directory: {}",
            lib_dir.display()
        ));
    }

    let same_dir = canonical_or_original(lib_dir) == canonical_or_original(runtime_dir);
    if !same_dir {
        clear_staged_runtime_dlls(runtime_dir);
        for entry in fs::read_dir(lib_dir)
            .map_err(|e| format!("Failed to read {}: {}", lib_dir.display(), e))?
        {
            let entry =
                entry.map_err(|e| format!("Failed to inspect {}: {}", lib_dir.display(), e))?;
            let path = entry.path();
            if path.extension() != Some(OsStr::new("dll")) {
                continue;
            }

            let file_name = entry.file_name();
            let destination = runtime_dir.join(file_name);
            fs::copy(&path, &destination).map_err(|e| {
                format!(
                    "Failed to copy DirectML runtime DLL from {} to {}: {}",
                    path.display(),
                    destination.display(),
                    e
                )
            })?;
        }
    }

    let missing: Vec<&str> = DIRECTML_RUNTIME_FILES
        .iter()
        .copied()
        .filter(|name| !runtime_dir.join(name).is_file())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "DirectML runtime staging is incomplete; missing {} in {}",
            missing.join(", "),
            runtime_dir.display()
        ));
    }

    Ok(())
}

fn clear_staged_runtime_dlls(runtime_dir: &Path) {
    if let Ok(entries) = fs::read_dir(runtime_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension() == Some(OsStr::new("dll")) {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
