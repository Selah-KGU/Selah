//! macOS WidgetKit snapshot publisher.
//!
//! The extension reads \`widget-snapshot.json\`. A bundled Selah.app is the widget's
//! parent. \`tauri dev\` is a bare binary, so a small host app keeps the extension
//! registered and reloads timelines when the snapshot changes.

use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike, Weekday};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::time::UNIX_EPOCH;

use crate::config::PERIOD_TIMES;
use crate::db::Database;

const SNAPSHOT_NAME: &str = "widget-snapshot.json";
const HOST_APP_NAME: &str = "Selah Widget.app";
const WIDGET_BUNDLE_ID: &str = "com.kgu.selah.widget";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetClass {
    pub day: i32,
    pub period: i32,
    pub name: String,
    pub room: String,
    pub start_minutes: i32,
    pub end_minutes: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetTodo {
    pub title: String,
    pub course: String,
    pub due_unix: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetSnapshot {
    pub summary: String,
    pub classes: Vec<WidgetClass>,
    pub todos: Vec<WidgetTodo>,
}

#[derive(Debug, Clone)]
pub(crate) struct ClassSource {
    pub day: i32,
    pub period: i32,
    pub name: String,
    pub room: String,
    pub cancelled: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TodoSource {
    pub title: String,
    pub course: String,
    pub status: String,
    pub deadline: String,
}

pub fn publish(db: &Database) {
    if macos_major() < 14 {
        return;
    }
    match snapshot_from_db(db, Local::now()) {
        Ok(snapshot) => {
            if let Err(error) = write_snapshot(&snapshot) {
                log::warn!("widget snapshot write failed: {error}");
                return;
            }
            reload_timelines();
        }
        Err(error) => log::warn!("widget snapshot build failed: {error}"),
    }
}

pub fn ensure_host_registered() {
    #[cfg(target_os = "macos")]
    {
        if macos_major() < 14 || running_inside_widget_app() {
            return;
        }
        if let Err(error) = install_host() {
            log::warn!("widget host install failed: {error}");
        }
    }
}

fn snapshot_from_db(db: &Database, now: chrono::DateTime<Local>) -> Result<WidgetSnapshot, String> {
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        now.date_naive(),
    );
    let raw = db.build_raw_data(&scope.current, &scope.next, snap.luna_communities)?;
    let mut classes = classes_from_sources(
        &raw.kgc_entries_current
            .iter()
            .map(|row| ClassSource {
                day: row.day,
                period: row.period,
                name: row.name.clone(),
                room: row.room.clone(),
                cancelled: row.is_cancelled,
            })
            .collect::<Vec<_>>(),
    );
    if classes.is_empty() {
        classes = classes_from_sources(
            &raw.luna_courses
                .iter()
                .map(|row| ClassSource {
                    day: row.day,
                    period: row.period,
                    name: row.name.clone(),
                    room: String::new(),
                    cancelled: false,
                })
                .collect::<Vec<_>>(),
        );
    }
    let todos = todos_from_cache(db, now)?;
    Ok(assemble_snapshot(now, classes, todos))
}

fn todos_from_cache(
    db: &Database,
    now: chrono::DateTime<Local>,
) -> Result<Vec<WidgetTodo>, String> {
    let Some((json, _)) = db.get_data_cache("luna_todo")? else {
        return Ok(Vec::new());
    };
    let items: Vec<crate::luna_parser::LunaTodoItem> =
        serde_json::from_str(&json).unwrap_or_default();
    Ok(todos_from_sources(
        now,
        &items
            .into_iter()
            .map(|item| TodoSource {
                title: item.content_name,
                course: item.course_name,
                status: item.status,
                deadline: item.deadline,
            })
            .collect::<Vec<_>>(),
    ))
}

pub(crate) fn classes_from_sources(sources: &[ClassSource]) -> Vec<WidgetClass> {
    let mut classes = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for source in sources {
        if source.cancelled || source.day < 1 || source.day > 6 || source.period < 1 {
            continue;
        }
        let name = source.name.trim();
        if name.is_empty() {
            continue;
        }
        let key = (source.day, source.period, name.to_string());
        if !seen.insert(key) {
            continue;
        }
        let (start, end) = period_minutes(source.period);
        classes.push(WidgetClass {
            day: source.day,
            period: source.period,
            name: name.to_string(),
            room: source.room.trim().to_string(),
            start_minutes: start,
            end_minutes: end,
        });
    }
    classes.sort_by_key(|item| (item.day, item.period, item.name.clone()));
    classes
}

pub(crate) fn todos_from_sources(
    now: chrono::DateTime<Local>,
    sources: &[TodoSource],
) -> Vec<WidgetTodo> {
    let mut todos = Vec::new();
    for source in sources {
        if is_completed_status(&source.status) {
            continue;
        }
        let Some(due_unix) = parse_deadline(&source.deadline, now) else {
            continue;
        };
        if due_unix < now.timestamp() {
            continue;
        }
        let title = todo_title(&source.title, &source.course);
        if title.is_empty() {
            continue;
        }
        todos.push(WidgetTodo {
            title,
            course: source.course.trim().to_string(),
            due_unix,
        });
    }
    todos.sort_by_key(|item| item.due_unix);
    todos
}

pub(crate) fn assemble_snapshot(
    now: chrono::DateTime<Local>,
    classes: Vec<WidgetClass>,
    todos: Vec<WidgetTodo>,
) -> WidgetSnapshot {
    let today = weekday_number(now.date_naive());
    let now_minutes = now.hour() as i32 * 60 + now.minute() as i32;
    let remaining = classes
        .iter()
        .filter(|item| item.day == today && item.end_minutes > now_minutes)
        .count();
    let summary = if classes.iter().any(|item| item.day == today) {
        if remaining == 0 {
            "今日の授業はすべて終了".to_string()
        } else {
            format!("今日はあと{remaining}コマ")
        }
    } else if let Some(summary) = next_class(&classes, today).and_then(next_class_summary) {
        summary
    } else if classes.is_empty() && todos.is_empty() {
        "Selah を開くと表示されます".to_string()
    } else {
        "今日は授業がありません".to_string()
    };
    WidgetSnapshot {
        summary,
        classes,
        todos,
    }
}

fn next_class(classes: &[WidgetClass], today: i32) -> Option<&WidgetClass> {
    classes
        .iter()
        .filter(|item| (1..=6).contains(&item.day) && item.day != today)
        .min_by(|left, right| {
            class_distance(left.day, today)
                .cmp(&class_distance(right.day, today))
                .then(left.start_minutes.cmp(&right.start_minutes))
                .then(left.period.cmp(&right.period))
                .then(left.name.cmp(&right.name))
        })
}

fn class_distance(day: i32, today: i32) -> i32 {
    if day > today {
        day - today
    } else {
        day + 7 - today
    }
}

fn next_class_summary(class: &WidgetClass) -> Option<String> {
    let label = crate::config::DAY_SHORT
        .get(class.day as usize)
        .copied()
        .filter(|label| !label.is_empty())?;
    let hour = class.start_minutes.div_euclid(60);
    let minute = class.start_minutes.rem_euclid(60);
    Some(format!("次は{label}曜 {hour}:{minute:02}"))
}

fn period_minutes(period: i32) -> (i32, i32) {
    let index = (period - 1) as usize;
    let Some(&(start_h, start_m, end_h, end_m)) = PERIOD_TIMES.get(index) else {
        return (0, 0);
    };
    (
        (start_h as i32) * 60 + start_m as i32,
        (end_h as i32) * 60 + end_m as i32,
    )
}

fn weekday_number(date: NaiveDate) -> i32 {
    match date.weekday() {
        Weekday::Mon => 1,
        Weekday::Tue => 2,
        Weekday::Wed => 3,
        Weekday::Thu => 4,
        Weekday::Fri => 5,
        Weekday::Sat => 6,
        Weekday::Sun => 7,
    }
}

fn is_completed_status(status: &str) -> bool {
    let text = status.trim();
    text.contains("提出済")
        || text.contains("完了")
        || text.contains("採点済")
        || text.contains("受験済")
}

fn todo_title(content: &str, course: &str) -> String {
    let content = content.trim();
    let course = course.trim();
    let generic = matches!(
        content,
        "" | "課題"
            | "レポート"
            | "テスト"
            | "小テスト"
            | "掲示板"
            | "アンケート"
            | "出席"
            | "その他"
            | "未設定"
    );
    if !generic {
        return content.to_string();
    }
    if !course.is_empty() && !content.is_empty() {
        return format!("{course} {content}");
    }
    if !course.is_empty() {
        return course.to_string();
    }
    content.to_string()
}

pub(crate) fn parse_deadline(text: &str, now: chrono::DateTime<Local>) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let date = find_date(&chars)?;
    let time = find_time(&chars);
    let (hour, minute, second) = time.unwrap_or((23, 59, 59));
    let local = Local
        .with_ymd_and_hms(date.0, date.1, date.2, hour, minute, second)
        .single()?;
    let _ = now;
    Some(local.timestamp())
}

fn find_date(chars: &[char]) -> Option<(i32, u32, u32)> {
    let mut index = 0;
    while index + 8 <= chars.len() {
        if let Some(parsed) = date_at(chars, index) {
            return Some(parsed);
        }
        index += 1;
    }
    None
}

fn date_at(chars: &[char], index: usize) -> Option<(i32, u32, u32)> {
    let year = four_digits(chars, index)?;
    let separator = *chars.get(index + 4)?;
    if separator != '/' && separator != '-' {
        return None;
    }
    let (month, month_len) = number_at(chars, index + 5, 2)?;
    let after_month = index + 5 + month_len;
    let month_separator = *chars.get(after_month)?;
    if month_separator != '/' && month_separator != '-' && month_separator != '月' {
        return None;
    }
    let (day, _) = number_at(chars, after_month + 1, 2)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

fn four_digits(chars: &[char], index: usize) -> Option<i32> {
    let slice = chars.get(index..index + 4)?;
    if !slice.iter().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let text: String = slice.iter().collect();
    text.parse().ok()
}

fn number_at(chars: &[char], index: usize, max_digits: usize) -> Option<(u32, usize)> {
    let mut digits = String::new();
    for ch in chars.get(index..)?.iter().take(max_digits) {
        if !ch.is_ascii_digit() {
            break;
        }
        digits.push(*ch);
    }
    if digits.is_empty() {
        return None;
    }
    Some((digits.parse().ok()?, digits.len()))
}

fn find_time(chars: &[char]) -> Option<(u32, u32, u32)> {
    let text: String = chars.iter().collect();
    let mut rest = text.as_str();
    while let Some(index) = rest.find(':') {
        let before = &rest[..index];
        let hour_digits: String = before
            .chars()
            .rev()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let after = &rest[index + 1..];
        if let (Some((hour, _)), Some((minute, _))) =
            (take_number(&hour_digits, 2), take_number(after, 2))
        {
            if hour <= 23 && minute <= 59 {
                return Some((hour, minute, 0));
            }
        }
        rest = &rest[index + 1..];
    }
    None
}

fn take_number(text: &str, max_digits: usize) -> Option<(u32, usize)> {
    let digits: String = text
        .chars()
        .take(max_digits)
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    Some((digits.parse().ok()?, digits.len()))
}

fn write_snapshot(snapshot: &WidgetSnapshot) -> Result<(), String> {
    let json = serde_json::to_string_pretty(snapshot).map_err(|error| error.to_string())?;
    for path in snapshot_paths() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        write_atomic(&path, json.as_bytes())?;
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp).map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().ok();
    }
    fs::rename(&tmp, path).map_err(|error| error.to_string())
}

fn snapshot_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(data) = dirs::data_dir() {
        paths.push(data.join("com.kgu.selah").join(SNAPSHOT_NAME));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(
            home.join("Library/Group Containers/group.com.kgu.selah")
                .join(SNAPSHOT_NAME),
        );
        paths.push(
            home.join("Library/Containers")
                .join(WIDGET_BUNDLE_ID)
                .join("Data/Library/Application Support/com.kgu.selah")
                .join(SNAPSHOT_NAME),
        );
    }
    paths
}

fn reload_timelines() {
    #[cfg(target_os = "macos")]
    {
        if let Some(api) = widget_api() {
            unsafe { (api.reload)() };
        }
    }
}

#[cfg(target_os = "macos")]
fn running_inside_widget_app() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(parent) = exe.parent() else {
        return false;
    };
    parent.join("../PlugIns/SelahWidget.appex").exists()
}

#[cfg(target_os = "macos")]
fn install_host() -> Result<(), String> {
    let appex = PathBuf::from(env!("SELAH_WIDGET_APPEX"));
    let host_bin = PathBuf::from(env!("SELAH_WIDGET_HOST"));
    if !appex.is_dir() || !host_bin.is_file() {
        return Err("widget bundle was not built".into());
    }
    let dest = applications_dir()?.join(HOST_APP_NAME);
    let stamp = dest.join("Contents/Resources/widget-stamp");
    let source_stamp = stamp_value(&appex.join("Contents/MacOS/SelahWidget"))?;
    if stamp.is_file() && fs::read_to_string(&stamp).ok().as_deref() == Some(source_stamp.as_str())
    {
        launch_host(&dest);
        return Ok(());
    }
    if dest.exists() {
        let _ = Command::new("pkill")
            .args(["-f", "selah-widget-host"])
            .status();
        let mut last_error = String::from("widget host is still running");
        for _ in 0..20 {
            match fs::remove_dir_all(&dest) {
                Ok(()) => {
                    last_error.clear();
                    break;
                }
                Err(error) => {
                    last_error = error.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
        if dest.exists() {
            return Err(last_error);
        }
    }
    let plugins = dest.join("Contents/PlugIns");
    let macos = dest.join("Contents/MacOS");
    let resources = dest.join("Contents/Resources");
    fs::create_dir_all(&plugins).map_err(|error| error.to_string())?;
    fs::create_dir_all(&macos).map_err(|error| error.to_string())?;
    fs::create_dir_all(&resources).map_err(|error| error.to_string())?;
    copy_dir(&appex, &plugins.join("SelahWidget.appex"))?;
    fs::copy(&host_bin, macos.join("selah-widget-host")).map_err(|error| error.to_string())?;
    let icon = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("icons/icon.icns");
    if icon.is_file() {
        let _ = fs::copy(icon, resources.join("icon.icns"));
    }
    fs::write(dest.join("Contents/Info.plist"), host_info_plist())
        .map_err(|error| error.to_string())?;
    fs::write(&stamp, &source_stamp).map_err(|error| error.to_string())?;
    sign_host(&dest)?;
    register_host(&dest)?;
    launch_host(&dest);
    log::info!("registered macOS widget host at {}", dest.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn applications_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("home directory is unavailable")?;
    let dir = home.join("Applications");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

#[cfg(target_os = "macos")]
fn stamp_value(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|error| error.to_string())?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|time| time.as_secs())
        .unwrap_or(0);
    Ok(format!("{}:{}", meta.len(), modified))
}

#[cfg(target_os = "macos")]
fn host_info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>ja</string>
    <key>CFBundleExecutable</key><string>selah-widget-host</string>
    <key>CFBundleIdentifier</key><string>com.kgu.selah.widget-host</string>
    <key>CFBundleName</key><string>Selah</string>
    <key>CFBundleDisplayName</key><string>Selah</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>1.1.0</string>
    <key>CFBundleVersion</key><string>1.1.0</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSUIElement</key><true/>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "macos")]
fn sign_host(app: &Path) -> Result<(), String> {
    let entitlements =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("swift/widget/Widget.entitlements");
    let appex = app.join("Contents/PlugIns/SelahWidget.appex");
    run_status(
        "codesign",
        &[
            "--force",
            "--sign",
            "-",
            "--entitlements",
            &entitlements.to_string_lossy(),
            &appex.to_string_lossy(),
        ],
    )?;
    run_status(
        "codesign",
        &["--force", "--sign", "-", &app.to_string_lossy()],
    )
}

#[cfg(target_os = "macos")]
fn register_host(app: &Path) -> Result<(), String> {
    let appex = app.join("Contents/PlugIns/SelahWidget.appex");
    let _ = Command::new("pluginkit")
        .args(["-a", &appex.to_string_lossy()])
        .status();
    let _ = Command::new("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister")
        .args(["-f", &app.to_string_lossy()])
        .status();
    let _ = WIDGET_BUNDLE_ID;
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_host(app: &Path) {
    if host_is_running() {
        return;
    }
    let _ = Command::new("open")
        .args(["-g", "-j", &app.to_string_lossy()])
        .status();
}

#[cfg(target_os = "macos")]
fn host_is_running() -> bool {
    Command::new("pgrep")
        .args(["-f", "selah-widget-host"])
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(dest).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(src).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = dest.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_status(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("failed to run {program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed"))
    }
}

fn macos_major() -> u32 {
    crate::local_ai_support::macos_major_version().unwrap_or(0)
}

#[cfg(target_os = "macos")]
type ReloadFn = unsafe extern "C" fn();

#[derive(Clone, Copy)]
#[cfg(target_os = "macos")]
struct WidgetApi {
    reload: ReloadFn,
}

#[cfg(target_os = "macos")]
fn widget_api() -> Option<WidgetApi> {
    static LOADED: std::sync::OnceLock<Option<WidgetApi>> = std::sync::OnceLock::new();
    LOADED.get_or_init(load_widget_api).clone()
}

#[cfg(target_os = "macos")]
fn load_widget_api() -> Option<WidgetApi> {
    let path = widget_library_path()?;
    let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let handle = unsafe { dlopen(c_path.as_ptr(), 2 | 4) };
    if handle.is_null() {
        return None;
    }
    let name = std::ffi::CString::new("selah_widget_reload").ok()?;
    let symbol = unsafe { dlsym(handle, name.as_ptr()) };
    if symbol.is_null() {
        return None;
    }
    Some(WidgetApi {
        reload: unsafe { std::mem::transmute_copy(&symbol) },
    })
}

#[cfg(target_os = "macos")]
fn widget_library_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("SELAH_WIDGET_LIB") {
        candidates.push(PathBuf::from(path));
    }
    candidates.push(PathBuf::from(env!("SELAH_WIDGET_LIB")));
    candidates.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/lib/libselah_widget.dylib"
    )));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("../Frameworks/libselah_widget.dylib"));
            candidates.push(parent.join("libselah_widget.dylib"));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(target_os = "macos")]
extern "C" {
    fn dlopen(path: *const std::os::raw::c_char, flags: i32) -> *mut std::ffi::c_void;
    fn dlsym(
        handle: *mut std::ffi::c_void,
        symbol: *const std::os::raw::c_char,
    ) -> *mut std::ffi::c_void;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> chrono::DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 3, 16, 0, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn saturday_classes_keep_period_order_and_drop_cancelled() {
        let classes = classes_from_sources(&[
            ClassSource {
                day: 6,
                period: 4,
                name: "英語".into(),
                room: "A1".into(),
                cancelled: false,
            },
            ClassSource {
                day: 6,
                period: 3,
                name: "情報".into(),
                room: "B2".into(),
                cancelled: false,
            },
            ClassSource {
                day: 6,
                period: 1,
                name: "休講科目".into(),
                room: "".into(),
                cancelled: true,
            },
            ClassSource {
                day: 6,
                period: 3,
                name: "情報".into(),
                room: "B2".into(),
                cancelled: false,
            },
        ]);
        assert_eq!(classes.len(), 2);
        assert_eq!(classes[0].period, 3);
        assert_eq!(classes[0].start_minutes, 13 * 60 + 30);
        assert_eq!(classes[1].name, "英語");
    }

    #[test]
    fn upcoming_todos_are_not_capped_at_five() {
        let sources: Vec<TodoSource> = (1..=6)
            .map(|day| TodoSource {
                title: format!("課題{day}"),
                course: "情報".into(),
                status: "未提出".into(),
                deadline: format!("2026/10/{:02} 12:00", day + 3),
            })
            .collect();
        assert_eq!(todos_from_sources(now(), &sources).len(), 6);
    }

    #[test]
    fn deadlines_skip_completed_and_past_items() {
        let todos = todos_from_sources(
            now(),
            &[
                TodoSource {
                    title: "済".into(),
                    course: "数学".into(),
                    status: "提出済み".into(),
                    deadline: "2026/10/05 23:59".into(),
                },
                TodoSource {
                    title: "古い".into(),
                    course: "数学".into(),
                    status: "未提出".into(),
                    deadline: "2026/10/01 09:00".into(),
                },
                TodoSource {
                    title: "レポート".into(),
                    course: "情報".into(),
                    status: "未提出".into(),
                    deadline: "2026/10/05 17:00".into(),
                },
                TodoSource {
                    title: "課題".into(),
                    course: "英語".into(),
                    status: "未提出".into(),
                    deadline: "2026-10-04".into(),
                },
            ],
        );
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].title, "英語 課題");
        assert_eq!(todos[1].title, "情報 レポート");
        assert!(todos[0].due_unix < todos[1].due_unix);
    }

    #[test]
    fn summary_counts_remaining_saturday_classes() {
        let classes = classes_from_sources(&[
            ClassSource {
                day: 6,
                period: 1,
                name: "朝".into(),
                room: "".into(),
                cancelled: false,
            },
            ClassSource {
                day: 6,
                period: 5,
                name: "夕".into(),
                room: "".into(),
                cancelled: false,
            },
        ]);
        let snapshot = assemble_snapshot(now(), classes, Vec::new());
        assert_eq!(snapshot.summary, "今日はあと1コマ");
    }

    #[test]
    fn summary_rolls_to_monday_when_saturday_has_no_class() {
        let classes = classes_from_sources(&[
            ClassSource {
                day: 1,
                period: 1,
                name: "英語".into(),
                room: "".into(),
                cancelled: false,
            },
            ClassSource {
                day: 2,
                period: 2,
                name: "法学".into(),
                room: "".into(),
                cancelled: false,
            },
        ]);
        let snapshot = assemble_snapshot(now(), classes, Vec::new());
        assert_eq!(snapshot.summary, "次は月曜 9:00");
    }

    #[test]
    fn summary_prefers_later_this_week_before_wrapping() {
        let thursday = Local
            .with_ymd_and_hms(2026, 10, 1, 8, 0, 0)
            .single()
            .unwrap();
        assert_eq!(thursday.weekday(), Weekday::Thu);
        let classes = classes_from_sources(&[
            ClassSource {
                day: 1,
                period: 1,
                name: "月曜".into(),
                room: "".into(),
                cancelled: false,
            },
            ClassSource {
                day: 5,
                period: 1,
                name: "金曜".into(),
                room: "".into(),
                cancelled: false,
            },
        ]);
        let snapshot = assemble_snapshot(thursday, classes, Vec::new());
        assert_eq!(snapshot.summary, "次は金曜 9:00");
    }

    #[test]
    fn summary_keeps_finished_copy_when_today_already_ended() {
        let classes = classes_from_sources(&[
            ClassSource {
                day: 6,
                period: 1,
                name: "朝".into(),
                room: "".into(),
                cancelled: false,
            },
            ClassSource {
                day: 1,
                period: 1,
                name: "英語".into(),
                room: "".into(),
                cancelled: false,
            },
        ]);
        let snapshot = assemble_snapshot(now(), classes, Vec::new());
        assert_eq!(snapshot.summary, "今日の授業はすべて終了");
    }

    #[test]
    fn deadline_with_multibyte_prefix_keeps_character_boundaries() {
        let due = parse_deadline("締切は2026/10/05 17:00です", now()).unwrap();
        let expected = Local
            .with_ymd_and_hms(2026, 10, 5, 17, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        assert_eq!(due, expected);
    }
}
