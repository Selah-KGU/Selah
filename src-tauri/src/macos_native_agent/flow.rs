use super::*;

// ─ Public setup ──────────────────────────────────────────────────────────────
pub fn setup(app: &AppHandle) {
    SYSTEM_IS_DARK.store(effective_is_dark(app), Ordering::Relaxed);

    let app_theme = app.clone();
    let _ = app.run_on_main_thread(move || register_appearance_observer(app_theme));

    let app_theme_event = app.clone();
    app.listen("app-theme-changed", move |_event| {
        let dark = effective_is_dark(&app_theme_event);
        SYSTEM_IS_DARK.store(dark, Ordering::Relaxed);
        let app_main = app_theme_event.clone();
        let _ = app_main.run_on_main_thread(move || apply_theme(&Theme::current()));
    });

    crate::native_agent_events::install(
        &SHARED,
        enqueue_view_update,
        finish_capture,
        capture_error,
    );
}

// ─ Agent submission ──────────────────────────────────────────────────────────
pub(super) fn capture_error(app: &AppHandle, input_id: &str, message: &str) {
    crate::native_agent_submission::capture_failed(
        app,
        input_id,
        message,
        &SHARED,
        enqueue_view_update,
    );
}

pub(super) fn finish_capture(app: AppHandle, input_id: &str) {
    finish_capture_owned(app, input_id, None);
}
pub(super) fn finish_capture_for_release(app: AppHandle, input_id: &str, token: u64) {
    finish_capture_owned(app, input_id, Some(token));
}
fn finish_capture_owned(app: AppHandle, input_id: &str, release_token: Option<u64>) {
    let (text, start) = {
        let mut sh = SHARED.lock().unwrap();
        let finished = match release_token {
            Some(token) => sh.finish_released_capture(
                input_id,
                token,
                RELEASE_FINALIZE_TOKEN.load(Ordering::Relaxed),
            ),
            None => sh.capture.finish(input_id),
        };
        if !finished {
            return;
        }
        sh.stop_requested = false;
        let text = consume_all_speech(&mut sh);
        if text.is_empty() {
            let view = sh.prepare_view(CapsuleMode::Listening, String::new());
            drop(sh);
            close_panel_if_current(&app, &view.lease);
            return;
        }
        // Reserve the conversation before releasing capture ownership. A new
        // shortcut can hide this result while its speech still reaches history.
        let conv_id = uuid_v4();
        let start = sh.begin_stream(conv_id);
        (text, start)
    };
    crate::native_agent_submission::submit(
        app.clone(),
        start.owner,
        text,
        &SHARED,
        enqueue_view_update,
    );
    enqueue_view_update(&app, start.view);
}

pub(super) fn clear_agent_stream() {
    SHARED.lock().unwrap().cancel_stream();
}

// ─ Result height measurement ─────────────────────────────────────────────────
pub(super) fn compute_result_height(text: &str) -> f64 {
    let text_w = RESULT_W - RESULT_PAD_X * 2.0;
    // Rough measurement — each character counted as ~font*0.56 (cjk ~font*0.98).
    let mut total_lines = 0.0_f64;
    for raw in text.split('\n') {
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            total_lines += 0.5;
            continue;
        }
        let (font_size, indent) = heading_metrics(trimmed);
        let content = strip_markdown_syntax(trimmed);
        if content.is_empty() {
            total_lines += 0.5;
            continue;
        }
        let glyph_w = font_size * 0.56;
        let cjk_w = font_size * 0.98;
        let eff: f64 = content
            .chars()
            .map(|c| if c.is_ascii() { glyph_w } else { cjk_w })
            .sum();
        let effective_w = (text_w - indent).max(1.0);
        let lines = (eff / effective_w).ceil().max(1.0);
        total_lines += lines;
    }
    let visible = total_lines.min(RESULT_MAX_VISIBLE_LINES as f64).max(2.0);
    let line_px = RESULT_BODY_FONT * RESULT_LINE_HEIGHT_MUL;
    let text_h = visible * line_px + RESULT_PAD_Y * 2.0 + 6.0;
    text_h.clamp(RESULT_MIN_H, RESULT_MAX_H)
}

fn heading_metrics(line: &str) -> (f64, f64) {
    if let Some(rest) = line.strip_prefix("### ") {
        let _ = rest;
        (RESULT_H3_FONT, 0.0)
    } else if let Some(rest) = line.strip_prefix("## ") {
        let _ = rest;
        (RESULT_H2_FONT, 0.0)
    } else if let Some(rest) = line.strip_prefix("# ") {
        let _ = rest;
        (RESULT_H1_FONT, 0.0)
    } else if line.trim_start().starts_with("- ")
        || line.trim_start().starts_with("* ")
        || line.trim_start().starts_with("• ")
    {
        (RESULT_BODY_FONT, 14.0)
    } else {
        (RESULT_BODY_FONT, 0.0)
    }
}

fn strip_markdown_syntax(line: &str) -> String {
    let trimmed = line
        .trim_start_matches("### ")
        .trim_start_matches("## ")
        .trim_start_matches("# ")
        .trim_start();
    let without_bullet = if let Some(rest) = trimmed.strip_prefix("- ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("• ") {
        rest
    } else {
        trimmed
    };
    let mut out = String::with_capacity(without_bullet.len());
    let mut skip = false;
    for c in without_bullet.chars() {
        if c == '*' || c == '_' || c == '`' {
            skip = !skip;
            continue;
        }
        out.push(c);
    }
    out
}

pub(super) fn schedule_close(app: AppHandle, delay: Duration, lease: NativeViewLease) {
    let close = crate::main_thread_animation::MainThreadAnimation::start(&AUTO_CLOSE_TOKEN);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        let app_close = app.clone();
        close
            .frame(&app, move || {
                close_panel_if_current(&app_close, &lease);
            })
            .await;
    });
}

pub(super) fn cancel_auto_close() {
    AUTO_CLOSE_TOKEN.fetch_add(1, Ordering::Relaxed);
}

fn register_appearance_observer(app: AppHandle) {
    let app_ptr = Box::into_raw(Box::new(app)) as usize;

    unsafe {
        let dnc_cls = AnyClass::get(c"NSDistributedNotificationCenter").unwrap();
        let dnc: *mut AnyObject = msg_send![dnc_cls, defaultCenter];
        let notif_name = NSString::from_str("AppleInterfaceThemeChangedNotification");

        let block = RcBlock::new(move |_notif: *mut AnyObject| {
            let app_ref = &*(app_ptr as *const AppHandle);
            // Only track macOS appearance when the user's app theme is "system".
            if app_theme_mode(app_ref) != "system" {
                return;
            }
            let app_clone = app_ref.clone();
            let _ = app_clone.run_on_main_thread(move || {
                SYSTEM_IS_DARK.store(is_dark_mode(), Ordering::Relaxed);
                apply_theme(&Theme::current());
            });
            // Notify other modules (subtitle overlay etc.) that the
            // effective dark/light state has changed even though the user
            // didn't manually flip the toggle. The same event name is used
            // for explicit user changes so listeners stay simple.
            let _ = app_ref.emit("app-theme-changed", ());
        });

        let _: () = msg_send![
            dnc,
            addObserverForName: &*notif_name,
            object: std::ptr::null::<AnyObject>(),
            queue: std::ptr::null::<AnyObject>(),
            usingBlock: &*block
        ];
    }
}

fn is_dark_mode() -> bool {
    unsafe {
        let cls = AnyClass::get(c"NSAppearance").unwrap();
        let current: *mut AnyObject = msg_send![cls, currentDrawingAppearance];
        if current.is_null() {
            return true;
        }
        let name: *const AnyObject = msg_send![current, name];
        if name.is_null() {
            return true;
        }
        let cstr: *const std::os::raw::c_char = msg_send![name, UTF8String];
        if cstr.is_null() {
            return true;
        }
        std::ffi::CStr::from_ptr(cstr)
            .to_string_lossy()
            .contains("Dark")
    }
}

fn app_theme_mode(app: &AppHandle) -> String {
    let state = app.state::<crate::ThemeState>();
    let guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
    guard.clone()
}

fn effective_is_dark(app: &AppHandle) -> bool {
    match app_theme_mode(app).as_str() {
        "light" => false,
        "dark" => true,
        _ => is_dark_mode(),
    }
}

pub(super) fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
