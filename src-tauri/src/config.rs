use chrono::Weekday;

/// Base URLs for all backend services
pub const KG_COURSE_BASE: &str = "https://kg-course.kwansei.ac.jp";
pub const LUNA_BASE: &str = "https://luna.kwansei.ac.jp";
pub const KWIC_BASE: &str = "https://kwic.kwansei.ac.jp";

/// SAML login entry points
pub const LUNA_SAML_URL: &str = "https://luna.kwansei.ac.jp/saml/login?disco=true";
pub const KWIC_SAML_URL: &str = "https://kwic.kwansei.ac.jp/saml/login?disco=true";

/// Microsoft Graph / OAuth
pub const MS_AUTHORITY: &str = "https://login.microsoftonline.com/common/oauth2/v2.0";
pub const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

/// Period time slots (start_hour, start_min, end_hour, end_min)
pub const PERIOD_TIMES: [(u32, u32, u32, u32); 7] = [
    (9, 0, 10, 30),   // 1限
    (11, 0, 12, 30),  // 2限
    (13, 30, 15, 0),  // 3限
    (15, 10, 16, 40), // 4限
    (16, 50, 18, 20), // 5限
    (18, 30, 20, 0),  // 6限
    (20, 10, 21, 40), // 7限
];

/// Day-of-week labels (shared by tray, timetable, commands)
pub fn day_label(day: &str) -> &str {
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

pub fn day_to_chrono_weekday(day: &str) -> Option<Weekday> {
    match day {
        "月" => Some(Weekday::Mon),
        "火" => Some(Weekday::Tue),
        "水" => Some(Weekday::Wed),
        "木" => Some(Weekday::Thu),
        "金" => Some(Weekday::Fri),
        "土" => Some(Weekday::Sat),
        _ => None,
    }
}

/// Day-of-week short labels indexed by day number (1=Mon .. 6=Sat). Index 0 is unused.
pub const DAY_SHORT: [&str; 7] = ["", "月", "火", "水", "木", "金", "土"];

/// Shared auth-required error messages
pub const KGC_AUTH_REQUIRED_MSG: &str = "ログインしてください";

/// Shared error messages used across multiple modules
pub const TOO_MANY_WINDOWS_MSG: &str = "開いているウィンドウが多すぎます。いくつか閉じてください。";
pub const MAIL_AUTH_REQUIRED_MSG: &str = "メールにログインしてください";
pub const MAIL_SESSION_EXPIRED_MSG: &str =
    "メールセッションが期限切れです。再ログインしてください。";
