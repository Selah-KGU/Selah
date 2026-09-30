use std::sync::{LazyLock, Mutex};

#[cfg(target_os = "macos")]
use std::process::{Child, Command, Stdio};

#[cfg(target_os = "macos")]
static CAFFEINATE_CHILD: LazyLock<Mutex<Option<Child>>> = LazyLock::new(|| Mutex::new(None));

#[cfg(target_os = "windows")]
struct WindowsSleepAssertion {
    stop_tx: std::sync::mpsc::Sender<()>,
    stopped_rx: std::sync::mpsc::Receiver<Result<(), String>>,
    thread: std::thread::JoinHandle<()>,
}

#[cfg(target_os = "windows")]
static WINDOWS_SLEEP_ASSERTION: LazyLock<Mutex<Option<WindowsSleepAssertion>>> =
    LazyLock::new(|| Mutex::new(None));

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

    let mut assertion = WINDOWS_SLEEP_ASSERTION
        .lock()
        .map_err(|e| format!("sleep assertion lock failed: {e}"))?;
    if assertion.is_some() {
        return Ok(());
    }

    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let (stop_tx, stop_rx) = std::sync::mpsc::channel();
    let (stopped_tx, stopped_rx) = std::sync::mpsc::sync_channel(1);
    let thread = std::thread::Builder::new()
        .name("selah-prevent-sleep".to_string())
        .spawn(move || {
            let previous = unsafe {
                SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)
            };
            if previous == 0 {
                let _ = ready_tx.send(Err("failed to set Windows execution state".to_string()));
                return;
            }

            if ready_tx.send(Ok(())).is_err() {
                unsafe {
                    SetThreadExecutionState(ES_CONTINUOUS);
                }
                return;
            }

            let _ = stop_rx.recv();
            let cleared = unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            let result = if cleared == 0 {
                Err("failed to clear Windows execution state".to_string())
            } else {
                Ok(())
            };
            let _ = stopped_tx.send(result);
        })
        .map_err(|e| format!("failed to start Windows sleep assertion thread: {e}"))?;

    match ready_rx.recv() {
        Ok(Ok(())) => {
            *assertion = Some(WindowsSleepAssertion {
                stop_tx,
                stopped_rx,
                thread,
            });
            Ok(())
        }
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(error) => {
            let _ = thread.join();
            Err(format!(
                "Windows sleep assertion thread stopped before startup: {error}"
            ))
        }
    }
}

#[cfg(target_os = "windows")]
fn stop_impl() -> Result<(), String> {
    let mut state = WINDOWS_SLEEP_ASSERTION
        .lock()
        .map_err(|e| format!("sleep assertion lock failed: {e}"))?;
    let Some(assertion) = state.take() else {
        return Ok(());
    };
    drop(state);

    assertion
        .stop_tx
        .send(())
        .map_err(|e| format!("failed to stop Windows sleep assertion thread: {e}"))?;
    let result = assertion
        .stopped_rx
        .recv()
        .map_err(|e| format!("Windows sleep assertion thread stopped unexpectedly: {e}"))?;
    assertion
        .thread
        .join()
        .map_err(|_| "Windows sleep assertion thread panicked".to_string())?;

    result
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn start_impl(_reason: String) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn stop_impl() -> Result<(), String> {
    Ok(())
}

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::{start_impl, stop_impl};

    #[test]
    fn sleep_assertion_is_idempotent_and_clears_on_owner_thread() {
        start_impl("test".to_string()).expect("start sleep assertion");
        start_impl("test again".to_string()).expect("start is idempotent");
        stop_impl().expect("stop sleep assertion");
        stop_impl().expect("stop is idempotent");
    }
}
