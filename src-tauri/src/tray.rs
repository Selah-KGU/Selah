use serde::Deserialize;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

/// Period time slots (start_hour, start_min, end_hour, end_min)
const PERIOD_TIMES: [(u32, u32, u32, u32); 7] = [
    (9, 0, 10, 30),   // 1限
    (11, 0, 12, 30),  // 2限
    (13, 30, 15, 0),  // 3限
    (15, 10, 16, 40), // 4限
    (16, 50, 18, 20), // 5限
    (18, 30, 20, 0),  // 6限
    (20, 10, 21, 40), // 7限
];

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

#[derive(Debug, Deserialize)]
pub struct TrayClassEntry {
    pub day: String,
    pub period: i32,
    pub course_name: String,
    pub room: String,
    pub is_cancelled: bool,
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItemBuilder::with_id("show", "Selah を表示").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "終了").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .icon_as_template(true)
        .tooltip("Selah")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Update the tray tooltip and menu with next class info
#[tauri::command]
pub fn update_tray(app: AppHandle, entries: Vec<TrayClassEntry>) -> Result<(), String> {
    use chrono::{Local, Datelike, Timelike};

    let now = Local::now();
    let current_weekday = now.weekday();
    let current_h = now.hour();
    let current_m = now.minute();
    let current_minutes = current_h * 60 + current_m;

    // Find the next class
    let mut best: Option<(&TrayClassEntry, i32, i32)> = None; // (entry, days_ahead, start_minutes)

    for entry in &entries {
        if entry.is_cancelled {
            continue;
        }
        let Some(entry_weekday) = day_to_chrono_weekday(&entry.day) else { continue };
        if entry.period < 1 || entry.period > 7 {
            continue;
        }
        let (sh, sm, _, _) = PERIOD_TIMES[(entry.period - 1) as usize];
        let start_minutes = (sh * 60 + sm) as i32;

        // Calculate days ahead (0 = today, 1 = tomorrow, ...)
        let entry_day_num = entry_weekday.num_days_from_monday() as i32;
        let current_day_num = current_weekday.num_days_from_monday() as i32;
        let mut days_ahead = entry_day_num - current_day_num;
        if days_ahead < 0 {
            days_ahead += 7;
        }
        // If same day but class already ended, push to next week
        if days_ahead == 0 {
            let (_, _, eh, em) = PERIOD_TIMES[(entry.period - 1) as usize];
            let end_minutes = (eh * 60 + em) as i32;
            if current_minutes as i32 >= end_minutes {
                days_ahead = 7;
            }
        }

        let is_better = match &best {
            None => true,
            Some((_, bd, bs)) => {
                days_ahead < *bd || (days_ahead == *bd && start_minutes < *bs)
            }
        };
        if is_better {
            best = Some((entry, days_ahead, start_minutes));
        }
    }

    let tray = app.tray_by_id("main-tray").ok_or("tray not found")?;

    if let Some((entry, days_ahead, start_minutes)) = best {
        let sh = start_minutes / 60;
        let sm = start_minutes % 60;
        let (_, _, eh, em) = PERIOD_TIMES[(entry.period - 1) as usize];

        let time_label = if days_ahead == 0 {
            // Currently in class or upcoming today
            if current_minutes as i32 >= start_minutes {
                format!("授業中 〜{}:{:02}", eh, em)
            } else {
                let diff = start_minutes - current_minutes as i32;
                if diff <= 60 {
                    format!("{}分後", diff)
                } else {
                    format!("今日 {}:{:02}", sh, sm)
                }
            }
        } else if days_ahead == 1 {
            format!("明日 {}:{:02}", sh, sm)
        } else {
            format!("{} {}:{:02}", day_label(&entry.day), sh, sm)
        };

        let tooltip = format!("次: {} ({})", entry.course_name, time_label);
        let _ = tray.set_tooltip(Some(&tooltip));

        // Rebuild menu with next class info
        let info_text = format!("{}", entry.course_name);
        let time_text = format!("{} {}限 {}:{:02}〜{}:{:02}", time_label, entry.period, sh, sm, eh, em);
        let room_text = if entry.room.is_empty() {
            String::new()
        } else {
            entry.room.clone()
        };

        let mut builder = MenuBuilder::new(&app);

        let info_item = MenuItemBuilder::with_id("info", &info_text).enabled(false).build(&app).map_err(|e| e.to_string())?;
        builder = builder.item(&info_item);

        let time_item = MenuItemBuilder::with_id("time", &time_text).enabled(false).build(&app).map_err(|e| e.to_string())?;
        builder = builder.item(&time_item);

        if !room_text.is_empty() {
            let room_item = MenuItemBuilder::with_id("room", &room_text).enabled(false).build(&app).map_err(|e| e.to_string())?;
            builder = builder.item(&room_item);
        }

        builder = builder.separator();

        let show_item = MenuItemBuilder::with_id("show", "Selah を表示").build(&app).map_err(|e| e.to_string())?;
        let quit_item = MenuItemBuilder::with_id("quit", "終了").build(&app).map_err(|e| e.to_string())?;
        builder = builder.item(&show_item).separator().item(&quit_item);

        let menu = builder.build().map_err(|e| e.to_string())?;
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;

        // Re-register menu event handler
        tray.on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        });
    } else {
        let _ = tray.set_tooltip(Some("Selah - 授業なし"));

        let no_class = MenuItemBuilder::with_id("noclass", "授業予定なし").enabled(false).build(&app).map_err(|e| e.to_string())?;
        let show_item = MenuItemBuilder::with_id("show", "Selah を表示").build(&app).map_err(|e| e.to_string())?;
        let quit_item = MenuItemBuilder::with_id("quit", "終了").build(&app).map_err(|e| e.to_string())?;

        let menu = MenuBuilder::new(&app)
            .item(&no_class)
            .separator()
            .item(&show_item)
            .separator()
            .item(&quit_item)
            .build()
            .map_err(|e| e.to_string())?;

        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
        tray.on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        });
    }

    Ok(())
}

// ============ Tray Status Cycling ============

/// Shared state for cycling tray title text
pub struct TrayStatusState {
    items: Mutex<Vec<String>>,
    index: Mutex<usize>,
    running: AtomicBool,
}

impl TrayStatusState {
    pub fn new() -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            index: Mutex::new(0),
            running: AtomicBool::new(false),
        }
    }
}

/// Start the background cycling thread (call once at setup)
pub fn start_tray_cycle(app: &AppHandle, state: Arc<TrayStatusState>) {
    if state.running.swap(true, Ordering::SeqCst) {
        return; // already running
    }
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(8));
            let items = state.items.lock().unwrap();
            if items.is_empty() {
                if let Some(tray) = app.tray_by_id("main-tray") {
                    let _ = tray.set_title(None::<&str>);
                }
                continue;
            }
            let mut idx = state.index.lock().unwrap();
            *idx = *idx % items.len();
            let text = format!(" {}", items[*idx]);
            *idx = (*idx + 1) % items.len();
            drop(items);
            drop(idx);
            if let Some(tray) = app.tray_by_id("main-tray") {
                let _ = tray.set_title(Some(&text));
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
    let mut current = state.items.lock().map_err(|e| e.to_string())?;
    *current = items;
    let mut idx = state.index.lock().map_err(|e| e.to_string())?;
    *idx = 0;
    Ok(())
}
