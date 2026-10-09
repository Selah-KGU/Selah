//! Overlay window thread, frame animation, and auto-close.

use super::*;

// ─ Window creation ────────────────────────────────────────────────────────────
pub(super) fn spawn_overlay_thread(app: &AppHandle) {
    let dark = prefers_dark(app);
    std::thread::spawn(move || unsafe {
        let class_name = wide_null(CLASS_NAME);
        let title = wide_null("Selah Agent Overlay");
        let hinstance = GetModuleHandleW(null());

        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(overlay_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: null_mut(),
        };
        let _ = RegisterClassExW(&wc);

        let work = work_area();
        let center_x = (work.left + work.right) / 2;
        let top_y = work.top + TOP_MARGIN;
        let x = center_x - LISTEN_W / 2;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            x,
            top_y,
            LISTEN_W,
            LISTEN_H,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        );

        if hwnd.is_null() {
            log::error!(
                "agent overlay: CreateWindowExW failed: {}",
                std::io::Error::last_os_error()
            );
            CREATING.store(false, Ordering::SeqCst);
            return;
        }

        let r = CORNER_RADIUS * 2;
        let rgn = CreateRoundRectRgn(0, 0, LISTEN_W + 1, LISTEN_H + 1, r, r);
        if !rgn.is_null() {
            if SetWindowRgn(hwnd, rgn, 1) == 0 {
                let _ = DeleteObject(rgn as _);
            }
        }
        let _ = SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
        ShowWindow(hwnd, SW_HIDE);
        UpdateWindow(hwnd);

        {
            let mut s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            s.hwnd = hwnd as RawHwnd;
            s.width = LISTEN_W;
            s.height = LISTEN_H;
            s.center_x = center_x;
            s.top_y = top_y;
            s.alpha = 0;
            s.text.clear();
            s.dark = dark;
            s.displayed_epoch = None;
            s.displayed_mode = MODE_NONE;
        }
        OVERLAY_HWND.store(hwnd as isize, Ordering::Relaxed);
        HWND_READY.store(true, Ordering::Release);
        CREATING.store(false, Ordering::SeqCst);

        // Install the LL keyboard hook on this thread so it runs inside our
        // GetMessage loop — bypasses IME/TSF which intercepts RegisterHotKey.
        if HOOK_VK.load(Ordering::Relaxed) == 0 {
            // The shortcut may have been disabled while CreateWindowExW was
            // running and force_destroy_panel therefore had no hwnd to close.
            DESTROYING.store(true, Ordering::SeqCst);
            let _ = PostMessageW(hwnd, WM_CLOSE, 0, 0);
        } else {
            install_ll_hook();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

pub(super) fn ensure_overlay_window(app: &AppHandle) {
    if HWND_READY.load(Ordering::Acquire) {
        return;
    }
    let _lock = CREATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if HWND_READY.load(Ordering::Relaxed)
        || CREATING.load(Ordering::SeqCst)
        || DESTROYING.load(Ordering::SeqCst)
    {
        return;
    }
    CREATING.store(true, Ordering::SeqCst);
    spawn_overlay_thread(app);
}

// ─ Animation ─────────────────────────────────────────────────────────────────
pub(super) fn morph_to(target_w: i32, target_h: i32, token: u64) {
    let animation = MainThreadAnimation::with_token(&MORPH_TOKEN, token);
    tauri::async_runtime::spawn(async move {
        let Some(Some(snap)) = animation.read_with(enqueue_ui_job, frame_snapshot).await else {
            return;
        };
        if snap.width == target_w && snap.height == target_h {
            return;
        }
        let mut sw = Spring::new(snap.width as f64);
        sw.set_target(target_w as f64);
        let mut sh = Spring::new(snap.height as f64);
        sh.set_target(target_h as f64);
        for _ in 0..90 {
            if !animation.is_current() {
                return;
            }
            let moving_w = sw.tick();
            let moving_h = sh.tick();
            let width = sw.pos.round() as i32;
            let height = sh.pos.round() as i32;
            if animation
                .read_with(enqueue_ui_job, move || {
                    apply_frame(width, height, snap.center_x, snap.top_y)
                })
                .await
                .is_none()
            {
                return;
            }
            if !moving_w && !moving_h {
                break;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        animation
            .read_with(enqueue_ui_job, move || {
                apply_frame(target_w, target_h, snap.center_x, snap.top_y)
            })
            .await;
    });
}

pub(super) fn fade_in(token: u64) {
    fade_to(255, token);
}
pub(super) fn fade_out_then_hide(token: u64) {
    fade_to(0, token);
}
fn fade_to(target: u8, token: u64) {
    let animation = MainThreadAnimation::with_token(&FADE_TOKEN, token);
    tauri::async_runtime::spawn(async move {
        let Some(Some(snap)) = animation.read_with(enqueue_ui_job, frame_snapshot).await else {
            return;
        };
        for i in 0..=FADE_FRAMES {
            let alpha = (snap.alpha as f64
                + (target as f64 - snap.alpha as f64)
                    * ease_out_quart(i as f64 / FADE_FRAMES as f64))
            .round()
            .clamp(0.0, 255.0) as u8;
            if animation
                .read_with(enqueue_ui_job, move || set_alpha(alpha))
                .await
                .is_none()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

// Called only when the feature is explicitly disabled (apply_config enabled=false).
pub(super) fn force_destroy_panel() {
    let hwnd = {
        let mut s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        let h = s.hwnd;
        s.hwnd = 0;
        s.alpha = 0;
        s.text.clear();
        h
    };
    HWND_READY.store(false, Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    DESTROYING.store(true, Ordering::SeqCst);
    unsafe {
        let _ = PostMessageW(hwnd_from_raw(hwnd), WM_CLOSE, 0, 0);
    }
}

pub(super) fn start_dots_animation(token: u64) {
    let animation = MainThreadAnimation::with_token(&DOTS_TOKEN, token);
    tauri::async_runtime::spawn(async move {
        let mut phase = 0;
        loop {
            let updated = animation
                .read_with(enqueue_ui_job, move || {
                    if current_mode() != MODE_PROCESSING {
                        return false;
                    }
                    DOTS_ACTIVE.store(phase % 3, Ordering::Relaxed);
                    let hwnd = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd;
                    if hwnd != 0 {
                        unsafe {
                            InvalidateRect(hwnd_from_raw(hwnd), null(), 1);
                        }
                    }
                    true
                })
                .await;
            if updated != Some(true) {
                break;
            }
            phase = phase.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(DOTS_PERIOD_MS)).await;
        }
        // A cancelled old task must not reset the new processing indicator.
        animation
            .read_with(enqueue_ui_job, || DOTS_ACTIVE.store(-1, Ordering::Relaxed))
            .await;
    });
}
pub(super) fn stop_dots_animation() {
    DOTS_TOKEN.fetch_add(1, Ordering::Relaxed);
    DOTS_ACTIVE.store(-1, Ordering::Relaxed);
}
pub(super) fn schedule_auto_close(delay: Duration, lease: NativeViewLease, token: u64) {
    let animation = MainThreadAnimation::with_token(&AUTO_CLOSE_TOKEN, token);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        animation
            .read_with(enqueue_ui_job, move || {
                if let Some(app) = APP_HANDLE.get() {
                    close_panel_for_view(app, &lease);
                }
            })
            .await;
    });
}

pub(super) fn cancel_auto_close() {
    AUTO_CLOSE_TOKEN.fetch_add(1, Ordering::Relaxed);
}

// ─ Navigation ─────────────────────────────────────────────────────────────────
pub(super) fn bring_main_window_to_front() {
    let Some(app) = APP_HANDLE.get() else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = crate::agent_commands::open_agent_popup(app, None, None, None, None).await;
    });
}
