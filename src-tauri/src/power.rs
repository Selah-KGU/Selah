use std::sync::{LazyLock, Mutex};

#[cfg(target_os = "macos")]
use std::process::{Child, Command, Stdio};

#[cfg(target_os = "macos")]
static CAFFEINATE_CHILD: LazyLock<Mutex<Option<Child>>> = LazyLock::new(|| Mutex::new(None));

#[cfg(target_os = "windows")]
static WINDOWS_SLEEP_ASSERTION: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));

#[tauri::command]
pub fn prevent_sleep_start(reason: Option<String>) -> Result<(), String> {
    start_impl(reason.unwrap_or_else(|| "KWIC live transcription".to_string()))
}

#[tauri::command]
pub fn prevent_sleep_stop() -> Result<(), String> {
    stop_impl()
}

#[cfg(target_os = "macos")]
fn start_impl(_reason: String) -> Result<(), String> {
    let mut child = CAFFEINATE_CHILD
        .lock()
        .map_err(|e| format!("sleep assertion lock failed: {e}"))?;

    if let Some(current) = child.as_mut() {
        match current.try_wait() {
            Ok(None) => return Ok(()),
            Ok(Some(_)) => {
                *child = None;
            }
            Err(_) => {
                let mut stale = child.take().expect("child existed");
                let _ = stale.kill();
                let _ = stale.wait();
            }
        }
    }

    let spawned = Command::new("caffeinate")
        .args(["-d", "-i", "-u"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to start caffeinate: {e}"))?;

    *child = Some(spawned);
    Ok(())
}

#[cfg(target_os = "macos")]
fn stop_impl() -> Result<(), String> {
    let mut child = CAFFEINATE_CHILD
        .lock()
        .map_err(|e| format!("sleep assertion lock failed: {e}"))?;

    if let Some(mut current) = child.take() {
        let _ = current.kill();
        let _ = current.wait();
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn start_impl(_reason: String) -> Result<(), String> {
    use windows_sys::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
    };

    let mut active = WINDOWS_SLEEP_ASSERTION
        .lock()
        .map_err(|e| format!("sleep assertion lock failed: {e}"))?;

    unsafe {
        let previous =
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED);
        if previous == 0 {
            return Err("failed to set Windows execution state".to_string());
        }
    }

    *active = true;
    Ok(())
}

#[cfg(target_os = "windows")]
fn stop_impl() -> Result<(), String> {
    use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS};

    let mut active = WINDOWS_SLEEP_ASSERTION
        .lock()
        .map_err(|e| format!("sleep assertion lock failed: {e}"))?;

    if *active {
        unsafe {
            let previous = SetThreadExecutionState(ES_CONTINUOUS);
            if previous == 0 {
                return Err("failed to clear Windows execution state".to_string());
            }
        }
        *active = false;
    }

    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn start_impl(_reason: String) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn stop_impl() -> Result<(), String> {
    Ok(())
}
