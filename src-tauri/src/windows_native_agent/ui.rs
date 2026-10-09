//! Coalesced views and acknowledged frames on the window's owning Win32 thread.
use super::*;
use crate::main_thread_animation::MainThreadJob;
use crate::owned_ui_jobs::OwnedUiJobs;

static JOBS: LazyLock<Mutex<OwnedUiJobs<RawHwnd>>> =
    LazyLock::new(|| Mutex::new(OwnedUiJobs::default()));
static VIEWS: LazyLock<LatestUiMailbox<WindowView>> = LazyLock::new(LatestUiMailbox::default);

use crate::native_agent_state::NativeViewRequest as WindowView;
struct ViewTicket(u64);
impl Drop for ViewTicket {
    fn drop(&mut self) {
        VIEWS.cancel(self.0);
    }
}

pub(super) fn enqueue_ui_job(job: MainThreadJob) -> Result<(), String> {
    // PostMessage never waits for WndProc. Hold this publication lock so a job
    // cannot enter the registry after WM_DESTROY has drained its HWND.
    let window = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    let hwnd = window.hwnd;
    if hwnd == 0 {
        return Err("Agent window is unavailable".into());
    }
    let ticket = JOBS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(hwnd, job);
    let posted = unsafe { PostMessageW(hwnd_from_raw(hwnd), WM_AGENT_UI_JOB, ticket as WPARAM, 0) };
    drop(window);
    if posted == 0 {
        let job = JOBS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take(hwnd, ticket);
        drop(job);
        return Err(format!(
            "Agent UI dispatch failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
pub(super) fn run_ui_job(hwnd: RawHwnd, ticket: u64) {
    let job = JOBS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take(hwnd, ticket);
    if let Some(job) = job {
        job();
    }
}
pub(super) fn drop_ui_jobs(hwnd: RawHwnd) {
    let jobs = JOBS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain_owner(hwnd);
    drop(jobs);
}
pub(super) fn clear_pending_views() {
    VIEWS.clear();
}

pub(super) fn enqueue_view(app: &AppHandle, view: NativeViewUpdate) {
    enqueue(app, WindowView::Update(view), true);
}
pub(super) fn enqueue_close(app: &AppHandle, lease: NativeViewLease, immediate: bool) {
    enqueue(app, WindowView::Close { lease, immediate }, false);
}
fn enqueue(app: &AppHandle, value: WindowView, create: bool) {
    let Some(ticket) = VIEWS.push(value) else {
        return;
    };
    if create {
        ensure_overlay_window(app);
    }
    tauri::async_runtime::spawn(async move {
        let ticket = ViewTicket(ticket);
        while !HWND_READY.load(Ordering::Acquire) {
            if !VIEWS.is_scheduled(ticket.0) {
                return;
            }
            if !CREATING.load(Ordering::SeqCst) {
                if HWND_READY.load(Ordering::Acquire) {
                    break;
                }
                return;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        if let Err(error) = enqueue_ui_job(Box::new(move || {
            apply_pending_view(ticket.0);
            drop(ticket);
        })) {
            log::warn!("Agent view dispatch failed: {error}");
        }
    });
}

pub(super) fn apply_pending_view(ticket: u64) {
    let Some(request) = VIEWS.take(ticket) else {
        return;
    };
    let (changed, morph, fade, dots, close) = {
        let _agent = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        if !request.is_current() {
            return;
        }
        let changed = match &request {
            WindowView::Update(view) => {
                WINDOW
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .displayed_epoch
                    != Some(view.epoch)
            }
            WindowView::Close { .. } => true,
        };
        if changed {
            stop_dots_animation();
            (
                true,
                MORPH_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1),
                FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1),
                DOTS_TOKEN.load(Ordering::Relaxed),
                AUTO_CLOSE_TOKEN
                    .fetch_add(1, Ordering::Relaxed)
                    .wrapping_add(1),
            )
        } else {
            (false, 0, 0, 0, 0)
        }
    };
    match request {
        WindowView::Close { immediate, .. } => {
            {
                let mut window = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
                window.displayed_epoch = None;
                window.displayed_mode = MODE_NONE;
            }
            if immediate {
                set_alpha(0);
            } else {
                fade_out_then_hide(fade);
            }
        }
        WindowView::Update(view) => {
            {
                let mut window = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
                window.displayed_epoch = Some(view.epoch);
                window.displayed_mode = mode_code(Some(view.mode));
            }
            let size = match view.mode {
                CapsuleMode::Listening => (LISTEN_W, LISTEN_H),
                CapsuleMode::Processing => (PROCESS_W, PROCESS_H),
                CapsuleMode::Result => (
                    RESULT_W,
                    if changed {
                        estimate_result_height(&view.text)
                    } else {
                        0
                    },
                ),
                CapsuleMode::Notice => (NOTICE_W, NOTICE_H),
            };
            update_text_content(view.text);
            if !changed {
                return;
            }
            // Already on the owning thread; construction never blocks an async worker.
            let hwnd = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd;
            if hwnd == 0 {
                return;
            }
            unsafe {
                ShowWindow(hwnd_from_raw(hwnd), SW_SHOWNOACTIVATE);
            }
            morph_to(size.0, size.1, morph);
            fade_in(fade);
            match view.mode {
                CapsuleMode::Processing => start_dots_animation(dots),
                CapsuleMode::Result => schedule_auto_close(
                    Duration::from_secs(RESULT_AUTO_CLOSE_SECS),
                    view.lease,
                    close,
                ),
                CapsuleMode::Notice => schedule_auto_close(
                    Duration::from_millis(NOTICE_AUTO_CLOSE_MS),
                    view.lease,
                    close,
                ),
                CapsuleMode::Listening => {}
            }
        }
    }
}
