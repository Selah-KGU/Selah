//! Keep the event loop alive through accepted input saves and queued config/DB work.
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

use crate::pending_persistence::INPUT_SAVES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitIntent {
    Quit(i32),
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Requested(ExitIntent),
    Draining(ExitIntent),
    Ready(ExitIntent),
}

struct ShutdownState(Mutex<Phase>);
static SHUTDOWN: ShutdownState = ShutdownState(Mutex::new(Phase::Idle));

impl ShutdownState {
    fn begin(&self, code: i32) -> bool {
        let mut phase = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let intent = match *phase {
            Phase::Idle => ExitIntent::Quit(code),
            Phase::Requested(intent) => intent,
            _ => return false,
        };
        *phase = Phase::Draining(intent);
        true
    }

    fn request_restart(&self) {
        let mut phase = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *phase = match *phase {
            Phase::Idle | Phase::Requested(_) => Phase::Requested(ExitIntent::Restart),
            Phase::Draining(_) => Phase::Draining(ExitIntent::Restart),
            Phase::Ready(_) => Phase::Ready(ExitIntent::Restart),
        };
    }

    fn ready(&self) -> ExitIntent {
        let mut phase = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Phase::Draining(intent) = *phase else {
            unreachable!("shutdown was not draining")
        };
        *phase = Phase::Ready(intent);
        intent
    }

    fn reset(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Idle;
    }

    fn phase(&self) -> Phase {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(crate) fn is_shutting_down() -> bool {
    SHUTDOWN.phase() != Phase::Idle
}

/// Tauri ignores prevent_exit for its restart code. Drain via an ordinary,
/// preventable exit request before asking Tauri to restart the process.
pub(crate) fn request_restart(app: &tauri::AppHandle) {
    SHUTDOWN.request_restart();
    app.exit(0);
}

/// Returns whether this exit was retained by the asynchronous save path.
pub(crate) fn defer_exit(
    app: &tauri::AppHandle,
    api: &tauri::ExitRequestApi,
    code: Option<i32>,
) -> bool {
    if code == Some(tauri::RESTART_EXIT_CODE) {
        if !matches!(SHUTDOWN.phase(), Phase::Ready(_)) {
            log::error!("restart bypassed the save coordinator");
        }
        return false;
    }
    match SHUTDOWN.phase() {
        Phase::Ready(ExitIntent::Quit(_)) => false,
        Phase::Ready(ExitIntent::Restart) => {
            api.prevent_exit();
            app.request_restart();
            true
        }
        Phase::Idle | Phase::Requested(_) | Phase::Draining(_) => {
            api.prevent_exit();
            if SHUTDOWN.begin(code.unwrap_or(0)) {
                crate::agent_commands::seal_mutations();
                crate::ai::seal_operations();
                crate::app_state::seal_cache();
                INPUT_SAVES.begin_shutdown();
                drain(app.clone());
            }
            true
        }
    }
}

fn drain(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let result = drain_before_exit(&app).await;
        match result {
            Ok(()) => {
                let intent = SHUTDOWN.ready();
                let code = match intent {
                    ExitIntent::Quit(code) => code,
                    ExitIntent::Restart => 0,
                };
                // Keep fullscreen exit on the same preventable path too.
                app.exit(code);
            }
            Err(error) => {
                log::error!("shutdown canceled because recording could not be saved: {error}");
                // Coalesce further quit requests until the error is dismissed.
                // The pending voice buffer remains available for the next retry.
                rfd::AsyncMessageDialog::new()
                    .set_title("Selah")
                    .set_description(format!("録音を保存できなかったため、終了を中止しました。\n{error}\n\n保存先の空き容量やアクセス権を確認して、もう一度終了してください。"))
                    .set_level(rfd::MessageLevel::Error)
                    .show().await;
                #[cfg(target_os = "macos")]
                crate::macos_fullscreen_exit::cancel_deferred_quit();
                crate::stt::stt_cancel_shutdown();
                INPUT_SAVES.reopen();
                crate::agent_commands::reopen_mutations();
                crate::ai::reopen_operations();
                crate::app_state::reopen_cache();
                SHUTDOWN.reset();
            }
        }
    });
}

async fn drain_before_exit(app: &tauri::AppHandle) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    crate::native_agent_submission::retry_failed(app);
    let state = app.state::<crate::live::LiveState>().inner().clone();
    drain_with(
        &INPUT_SAVES,
        || async {
            tokio::task::spawn_blocking(|| {
                crate::stt::stt_shutdown_for_exit(Duration::from_secs(1))
            })
            .await
            .map_err(|error| format!("音声停止処理失敗: {error}"))?
        },
        || async {
            tokio::join!(
                crate::agent_commands::drain_mutations(),
                crate::ai::drain_operations(),
                crate::app_state::drain_cache()
            );
        },
        || async move {
            tokio::task::spawn_blocking(move || state.persist_before_exit())
                .await
                .map_err(|error| format!("LIVE終了保存処理失敗: {error}"))?
        },
    )
    .await
}

async fn drain_with<
    S: Future<Output = Result<bool, String>>,
    M: Future<Output = ()>,
    W: Future<Output = Result<(), String>>,
>(
    saves: &crate::pending_persistence::PendingPersistence,
    mut stop: impl FnMut() -> S,
    mutations: impl FnOnce() -> M,
    write: impl FnOnce() -> W,
) -> Result<(), String> {
    // A timeout is a wait, not permission to discard pending decoder work.
    let mut waits = 0;
    loop {
        let stopped = stop().await?;
        if stopped {
            break;
        }
        waits += 1;
        if waits % 5 == 0 {
            log::info!("shutdown is waiting for captured speech to finish decoding");
        }
    }
    saves.seal();
    let (saved, ()) = tokio::join!(saves.drained(), mutations());
    saved?;
    write().await
}

#[cfg(test)]
#[path = "app_shutdown/tests.rs"]
mod tests;
