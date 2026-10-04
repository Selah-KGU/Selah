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
pub(super) fn morph_to(target_w: i32, target_h: i32) {
    let Some(snap) = frame_snapshot() else { return };
    let token = MORPH_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        let mut sw = Spring::new(snap.width as f64);
        sw.set_target(target_w as f64);
        let mut sh = Spring::new(snap.height as f64);
        sh.set_target(target_h as f64);

        for _ in 0..90 {
            if MORPH_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            let mw = sw.tick();
            let mh = sh.tick();
            apply_frame(
                sw.pos.round() as i32,
                sh.pos.round() as i32,
                snap.center_x,
                snap.top_y,
            );
            if !mw && !mh {
                break;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        if MORPH_TOKEN.load(Ordering::Relaxed) == token {
            apply_frame(target_w, target_h, snap.center_x, snap.top_y);
        }
    });
}

pub(super) fn fade_in() {
    let start = frame_snapshot().map(|s| s.alpha).unwrap_or(0);
    if start >= 250 {
        return;
    }
    let token = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        for i in 0..=FADE_FRAMES {
            if FADE_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            let a = start as f64
                + (255.0 - start as f64) * ease_out_quart(i as f64 / FADE_FRAMES as f64);
            set_alpha(a.round().clamp(0.0, 255.0) as u8);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

pub(super) fn fade_out_then_hide() {
    let token = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        for i in (0..=FADE_FRAMES).rev() {
            if FADE_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            let a = (255.0 * ease_out_quart(i as f64 / FADE_FRAMES as f64))
                .round()
                .clamp(0.0, 255.0) as u8;
            set_alpha(a);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        if FADE_TOKEN.load(Ordering::Relaxed) == token {
            hide_panel();
        }
    });
}

// Just hide the window — never destroy it while the shortcut is enabled,
// so the LL keyboard hook (which lives on the overlay thread) stays installed.
pub(super) fn hide_panel() {
    let hwnd = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd;
    if hwnd == 0 {
        return;
    }
    unsafe {
        ShowWindow(hwnd_from_raw(hwnd), SW_HIDE);
    }
    // HWND_READY stays true — the window still exists.
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
        ShowWindow(hwnd_from_raw(hwnd), SW_HIDE);
        let _ = PostMessageW(hwnd_from_raw(hwnd), WM_CLOSE, 0, 0);
    }
}

pub(super) fn start_dots_animation() {
    DOTS_ACTIVE.store(0, Ordering::Relaxed);
    let token = DOTS_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        let mut phase: i32 = 0;
        loop {
            if DOTS_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            if CURRENT_MODE.load(Ordering::Relaxed) != MODE_PROCESSING {
                break;
            }
            DOTS_ACTIVE.store(phase % 3, Ordering::Relaxed);
            let hwnd = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd;
            if hwnd != 0 {
                unsafe {
                    let _ = InvalidateRect(hwnd_from_raw(hwnd), null(), 1);
                }
            }
            phase = phase.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(DOTS_PERIOD_MS)).await;
        }
        DOTS_ACTIVE.store(-1, Ordering::Relaxed);
    });
}

pub(super) fn stop_dots_animation() {
    DOTS_TOKEN.fetch_add(1, Ordering::Relaxed);
    DOTS_ACTIVE.store(-1, Ordering::Relaxed);
}

pub(super) fn schedule_auto_close(delay: Duration) {
    let token = AUTO_CLOSE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        if AUTO_CLOSE_TOKEN.load(Ordering::Relaxed) == token {
            if let Some(app) = APP_HANDLE.get() {
                close_panel(app, false);
            }
        }
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
