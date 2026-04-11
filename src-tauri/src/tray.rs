use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tauri::{
    image::Image,
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState},
    AppHandle, Emitter, Manager, Position, Size, Rect,
};

use crate::config;

fn day_label(day: &str) -> &str {
    match day {
        "月" => "月曜",
        "火" => "火曜",
        "水" => "水曜",
        "木" => "木曜",
        "金" => "金曜",
        "土" => "土曜",
        _ => day,
    }
}

fn day_to_chrono_weekday(day: &str) -> Option<chrono::Weekday> {
    match day {
        "月" => Some(chrono::Weekday::Mon),
        "火" => Some(chrono::Weekday::Tue),
        "水" => Some(chrono::Weekday::Wed),
        "木" => Some(chrono::Weekday::Thu),
        "金" => Some(chrono::Weekday::Fri),
        "土" => Some(chrono::Weekday::Sat),
        _ => None,
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TrayClassEntry {
    pub day: String,
    pub period: i32,
    pub course_name: String,
    pub room: String,
    pub is_cancelled: bool,
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let icon = Image::from_bytes(include_bytes!("../icons/tray-icon@2x.png"))
        .map_err(|e| tauri::Error::AssetNotFound(format!("tray icon: {}", e)))?;
    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Selah")
        .on_tray_icon_event(|tray, event| {
            match event {
                TrayIconEvent::Click {
                    button: MouseButton::Right,
                    button_state: MouseButtonState::Up,
                    rect,
                    ..
                } => {
                    let app = tray.app_handle().clone();
                    if let Err(e) = toggle_tray_popup(&app, rect) {
                        log::warn!("tray popup open failed: {}", e);
                    }
                }
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    rect,
                    ..
                } => {
                    let app = tray.app_handle().clone();
                    if let Err(e) = toggle_tray_popup(&app, rect) {
                        log::warn!("tray popup open failed: {}", e);
                    }
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

/// Toggle tray popup: if exists close it, otherwise open it.
fn toggle_tray_popup(app: &AppHandle, icon_rect: Rect) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("tray-popup") {
        let _ = win.close();
        return Ok(());
    }

    let popup_w: f64 = 300.0;
    let popup_h: f64 = 400.0;

    let scale = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);

    // Convert icon rect to logical coords
    let (icon_x, icon_y) = match icon_rect.position {
        Position::Physical(p) => (p.x as f64 / scale, p.y as f64 / scale),
        Position::Logical(p) => (p.x, p.y),
    };
    let (icon_w, icon_h) = match icon_rect.size {
        Size::Physical(s) => (s.width as f64 / scale, s.height as f64 / scale),
        Size::Logical(s) => (s.width, s.height),
    };

    // Center popup horizontally below the tray icon
    let icon_center_x = icon_x + icon_w / 2.0;
    let mut x = (icon_center_x - popup_w / 2.0).max(4.0);
    // Place directly below the icon bottom edge
    let y = icon_y + icon_h + 4.0;

    // Clamp to screen right edge
    if let Some(monitor) = app.primary_monitor().ok().flatten() {
        let screen_w = monitor.size().width as f64 / scale;
        if x + popup_w > screen_w - 4.0 {
            x = screen_w - popup_w - 4.0;
        }
    }

    let win = tauri::WebviewWindowBuilder::new(
        app,
        "tray-popup",
        tauri::WebviewUrl::App("tray-popup.html".into()),
    )
    .title("")
    .inner_size(popup_w, popup_h)
    .position(x, y)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(true)
    .decorations(false)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .focused(true)
    .shadow(false)
    .background_color(tauri::webview::Color(0, 0, 0, 0))
    .build()
    .map_err(|e| format!("Popup creation failed: {}", e))?;

    // Auto-close on focus loss: emit event to JS for exit animation,
    // then force-close after a grace period as safety net.
    let app2 = app.clone();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            // Notify JS so it can run exit animation (more reliable than JS blur)
            if let Some(w) = app2.get_webview_window("tray-popup") {
                let _ = w.emit("popup-blur", ());
            }
            // Safety net: force-close after JS animation duration
            let app3 = app2.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if let Some(w) = app3.get_webview_window("tray-popup") {
                    let _ = w.close();
                }
            });
        }
    });

    // Re-apply z-order/workspace flags at runtime to avoid macOS fullscreen stacking glitches.
    let _ = win.set_always_on_bottom(false);
    let _ = win.set_visible_on_all_workspaces(true);
    let _ = win.set_always_on_top(true);

    let _ = win.show();
    let _ = win.set_focus();

    Ok(())
}

/// Show the main window (called from tray popup)
#[tauri::command]
pub fn show_main_window(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
    Ok(())
}

/// Quit app (called from tray popup)
#[tauri::command]
pub fn quit_app(app: AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

/// Data returned to tray popup for rendering
#[derive(Serialize)]
pub struct TrayPopupData {
    pub entries: Vec<TrayClassEntry>,
    pub todos: Vec<serde_json::Value>,
    pub student_id: String,
    pub student_name: String,
}

/// Provide data for the tray popup window (reads from DB cache)
#[tauri::command]
pub async fn get_tray_popup_data(
    db: tauri::State<'_, crate::db::Database>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<TrayPopupData, String> {
    // Read timetable entries from the cached schedule_data
    let entries: Vec<TrayClassEntry> = db.get_data_cache("schedule_data").ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|v| {
            v.get("raw")?.get("kgc_entries_current")
                .and_then(|arr| {
                    arr.as_array().map(|items| {
                        items.iter().filter_map(|item| {
                            let day_num = item.get("day")?.as_i64()? as i32;
                            let day_str = match day_num {
                                1 => "月", 2 => "火", 3 => "水", 4 => "木", 5 => "金", 6 => "土",
                                _ => return None,
                            };
                            Some(TrayClassEntry {
                                day: day_str.to_string(),
                                period: item.get("period")?.as_i64()? as i32,
                                course_name: item.get("name")?.as_str()?.to_string(),
                                room: item.get("room").and_then(|r| r.as_str()).unwrap_or("").to_string(),
                                is_cancelled: item.get("is_cancelled").and_then(|b| b.as_bool()).unwrap_or(false),
                            })
                        }).collect()
                    })
                })
        })
        .unwrap_or_default();

    // Read Luna TODOs from cache
    let todos: Vec<serde_json::Value> = db.get_data_cache("luna_todo").ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    let (student_id, student_name) = {
        let client = state.client.lock().await;
        if let Some(session) = &client.session {
            (session.student_id.clone(), session.display_name.clone())
        } else {
            (String::new(), String::new())
        }
    };

    Ok(TrayPopupData {
        entries,
        todos,
        student_id,
        student_name,
    })
}

/// Update the tray tooltip with next class info
#[tauri::command]
pub fn update_tray(app: AppHandle, entries: Vec<TrayClassEntry>) -> Result<(), String> {
    use chrono::{Local, Datelike, Timelike};

    let now = Local::now();
    let current_weekday = now.weekday();
    let current_h = now.hour();
    let current_m = now.minute();
    let current_minutes = current_h * 60 + current_m;

    // Find the next class
    let mut best: Option<(&TrayClassEntry, i32, i32)> = None;

    for entry in &entries {
        if entry.is_cancelled { continue; }
        let Some(entry_weekday) = day_to_chrono_weekday(&entry.day) else { continue };
        if entry.period < 1 || entry.period > 7 { continue; }
        let (sh, sm, _, _) = config::PERIOD_TIMES[(entry.period - 1) as usize];
        let start_minutes = (sh * 60 + sm) as i32;

        let entry_day_num = entry_weekday.num_days_from_monday() as i32;
        let current_day_num = current_weekday.num_days_from_monday() as i32;
        let mut days_ahead = entry_day_num - current_day_num;
        if days_ahead < 0 { days_ahead += 7; }
        if days_ahead == 0 {
            let (_, _, eh, em) = config::PERIOD_TIMES[(entry.period - 1) as usize];
            let end_minutes = (eh * 60 + em) as i32;
            if current_minutes as i32 >= end_minutes { days_ahead = 7; }
        }

        let is_better = match &best {
            None => true,
            Some((_, bd, bs)) => days_ahead < *bd || (days_ahead == *bd && start_minutes < *bs),
        };
        if is_better { best = Some((entry, days_ahead, start_minutes)); }
    }

    let tray = app.tray_by_id("main-tray").ok_or("tray not found")?;

    if let Some((entry, days_ahead, start_minutes)) = best {
        let sh = start_minutes / 60;
        let sm = start_minutes % 60;
        let (_, _, eh, em) = config::PERIOD_TIMES[(entry.period - 1) as usize];
        let end_minutes = eh as i32 * 60 + em as i32;

        let name: String = if entry.course_name.chars().count() > 18 {
            entry.course_name.chars().take(17).chain(std::iter::once('\u{2026}')).collect()
        } else {
            entry.course_name.clone()
        };

        let time_label = if days_ahead == 0 {
            if current_minutes as i32 >= start_minutes {
                let left = end_minutes - current_minutes as i32;
                format!("残{}分", left.max(0))
            } else {
                let diff = start_minutes - current_minutes as i32;
                if diff <= 60 { format!("{}分後", diff) }
                else { format!("今日 {}:{:02}", sh, sm) }
            }
        } else if days_ahead == 1 {
            format!("明日 {}:{:02}", sh, sm)
        } else {
            format!("{} {}:{:02}", day_label(&entry.day), sh, sm)
        };

        let _ = tray.set_tooltip(Some(&format!("{} | {}", time_label, name)));
    } else {
        let _ = tray.set_tooltip(Some("Selah"));
    }

    Ok(())
}

// ============ Tray Status Cycling ============

/// Shared state for cycling tray title text
pub struct TrayStatusState {
    inner: Mutex<(Vec<String>, usize)>,
    running: AtomicBool,
}

impl TrayStatusState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new((Vec::new(), 0)),
            running: AtomicBool::new(false),
        }
    }
}

/// Start the background cycling task (call once at setup)
pub fn start_tray_cycle(app: &AppHandle, state: Arc<TrayStatusState>) {
    if state.running.swap(true, Ordering::SeqCst) {
        return; // already running
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(8));
        interval.tick().await; // first tick fires immediately, skip it
        let mut last_text = String::new();
        loop {
            interval.tick().await;
            let Ok(mut guard) = state.inner.lock() else {
                continue; // mutex poisoned, skip this tick
            };
            let (ref items, ref mut idx) = *guard;
            if items.is_empty() {
                // Clear tray title when no items
                if !last_text.is_empty() {
                    last_text.clear();
                    drop(guard);
                    if let Some(tray) = app.tray_by_id("main-tray") {
                        let _ = tray.set_title(None::<&str>);
                    }
                }
                continue;
            }
            // Single item: set once, skip cycling
            if items.len() == 1 {
                let text = format!(" {}", items[0]);
                drop(guard);
                if text != last_text {
                    last_text = text.clone();
                    if let Some(tray) = app.tray_by_id("main-tray") {
                        let _ = tray.set_title(Some(&text));
                    }
                }
                continue;
            }
            *idx %= items.len();
            let text = format!(" {}", items[*idx]);
            *idx = (*idx + 1) % items.len();
            drop(guard);
            if text != last_text {
                last_text = text.clone();
                if let Some(tray) = app.tray_by_id("main-tray") {
                    let _ = tray.set_title(Some(&text));
                }
            }
        }
    });
}

/// Update the cycling status items from the frontend
#[tauri::command]
pub fn set_tray_status_items(
    state: tauri::State<'_, Arc<TrayStatusState>>,
    items: Vec<String>,
) -> Result<(), String> {
    let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
    guard.0 = items;
    guard.1 = 0;
    Ok(())
}
