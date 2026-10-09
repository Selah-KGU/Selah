//! Overlay painting and result-height estimation.

use super::*;

// ─ Result height estimation ───────────────────────────────────────────────────
pub(super) fn estimate_result_height(text: &str) -> i32 {
    let text_w = RESULT_W - RESULT_PAD_X * 2;
    let wide = wide_null(text);
    unsafe {
        let hdc = GetDC(null_mut());
        if !hdc.is_null() {
            let font = get_result_font();
            let old_font = SelectObject(hdc, font);
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: text_w,
                bottom: 0,
            };
            DrawTextW(
                hdc,
                wide.as_ptr(),
                -1,
                &mut rect,
                DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(hdc, old_font);
            ReleaseDC(null_mut(), hdc);
            return (rect.bottom + RESULT_PAD_Y * 2).clamp(RESULT_MIN_H, RESULT_MAX_H);
        }
    }
    RESULT_MIN_H
}

// ─ Paint ──────────────────────────────────────────────────────────────────────
pub(super) fn clear_destroyed_window(hwnd: RawHwnd) -> bool {
    let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if state.hwnd != hwnd {
        return false;
    }

    state.hwnd = 0;
    state.alpha = 0;
    state.text.clear();
    HWND_READY.store(false, Ordering::Release);
    true
}

pub(super) unsafe fn paint_overlay(hwnd: HWND) {
    let Some(state) = window_snapshot() else {
        return;
    };
    let mode = state.mode;

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    if hdc.is_null() {
        return;
    }

    // ── Colors ────────────────────────────────────────────────────────────────
    let bg = if state.dark {
        rgb(22, 20, 28)
    } else {
        rgb(250, 248, 253)
    };
    // Border: pre-blended purple on bg (≈28% opacity purple over bg)
    let border = if state.dark {
        rgb(63, 51, 85)
    } else {
        rgb(227, 209, 245)
    };
    let text_col = if state.dark {
        rgb(246, 246, 250)
    } else {
        rgb(30, 24, 54)
    };
    let muted_col = if state.dark {
        rgb(140, 136, 165)
    } else {
        rgb(150, 140, 170)
    };

    // ── Background + thin border via RoundRect ────────────────────────────────
    let brush = get_bg_brush(bg);
    let pen = CreatePen(PS_SOLID as i32, 1, border);
    let old_brush = SelectObject(hdc, brush);
    let old_pen = SelectObject(hdc, pen as HGDIOBJ);
    let r = CORNER_RADIUS * 2;
    RoundRect(hdc, 0, 0, state.width, state.height, r, r);

    // ── Mode-specific content ─────────────────────────────────────────────────
    match mode {
        MODE_LISTENING => {
            // Small indicator dot on the left
            let dot_col = if state.dark {
                rgb(255, 120, 158)
            } else {
                rgb(228, 78, 132)
            };
            let dot_brush = CreateSolidBrush(dot_col);
            let null_pen = GetStockObject(NULL_PEN_STOCK);
            let ob = SelectObject(hdc, dot_brush as HGDIOBJ);
            let op = SelectObject(hdc, null_pen);
            let dx = PAD_X - 4;
            let dy = (state.height - LISTEN_INDICATOR_SIZE) / 2;
            Ellipse(
                hdc,
                dx,
                dy,
                dx + LISTEN_INDICATOR_SIZE,
                dy + LISTEN_INDICATOR_SIZE,
            );
            SelectObject(hdc, op);
            SelectObject(hdc, ob);
            DeleteObject(dot_brush as HGDIOBJ);

            // Text — muted placeholder or transcribed speech
            let font = get_listen_font();
            let of = SelectObject(hdc, font);
            SetBkMode(hdc, TRANSPARENT as i32);
            let display = if state.text.is_empty() {
                MUTED_TEXT.to_string()
            } else {
                state.text.clone()
            };
            let is_muted = display == MUTED_TEXT;
            SetTextColor(hdc, if is_muted { muted_col } else { text_col });
            let wide = wide_null(&display);
            let gap = LISTEN_INDICATOR_SIZE + 8;
            let mut rect = RECT {
                left: PAD_X + gap,
                top: 0,
                right: state.width - PAD_X,
                bottom: state.height,
            };
            DrawTextW(
                hdc,
                wide.as_ptr(),
                -1,
                &mut rect,
                DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(hdc, of);
        }

        MODE_PROCESSING => {
            // Three sequential dots
            let dot_active = DOTS_ACTIVE.load(Ordering::Relaxed);
            let active_col = if state.dark {
                rgb(200, 154, 248)
            } else {
                rgb(150, 84, 220)
            };
            let dim_col = if state.dark {
                rgb(80, 64, 108)
            } else {
                rgb(210, 200, 225)
            };

            let total_w = DOT_SIZE * 3 + DOT_GAP * 2;
            let sx = (state.width - total_w) / 2;
            let sy = (state.height - DOT_SIZE) / 2;
            let null_pen = GetStockObject(NULL_PEN_STOCK);
            let op = SelectObject(hdc, null_pen);
            for i in 0i32..3 {
                let col = if dot_active == i { active_col } else { dim_col };
                let db = CreateSolidBrush(col);
                let ob = SelectObject(hdc, db as HGDIOBJ);
                let x = sx + i * (DOT_SIZE + DOT_GAP);
                Ellipse(hdc, x, sy, x + DOT_SIZE, sy + DOT_SIZE);
                SelectObject(hdc, ob);
                DeleteObject(db as HGDIOBJ);
            }
            SelectObject(hdc, op);
        }

        MODE_RESULT => {
            let font = get_result_font();
            let of = SelectObject(hdc, font);
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, text_col);
            let wide = wide_null(&state.text);
            let mut rect = RECT {
                left: RESULT_PAD_X,
                top: RESULT_PAD_Y,
                right: state.width - RESULT_PAD_X,
                bottom: state.height - RESULT_PAD_Y,
            };
            DrawTextW(
                hdc,
                wide.as_ptr(),
                -1,
                &mut rect,
                DT_WORDBREAK | DT_TOP | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            SelectObject(hdc, of);
        }

        MODE_NOTICE => {
            let font = get_notice_font();
            let of = SelectObject(hdc, font);
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, text_col);
            let wide = wide_null(&state.text);
            let mut rect = RECT {
                left: PAD_X,
                top: 0,
                right: state.width - PAD_X,
                bottom: state.height,
            };
            DrawTextW(
                hdc,
                wide.as_ptr(),
                -1,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(hdc, of);
        }

        _ => {}
    }

    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    DeleteObject(pen as HGDIOBJ);
    // brush is cached via get_bg_brush — do not delete here

    EndPaint(hwnd, &ps);
}
