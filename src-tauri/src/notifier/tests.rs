use super::*;
use crate::luna_parser::LunaNotification;

fn notif(content: &str) -> LunaNotification {
    LunaNotification {
        date: "2025/07/01".to_string(),
        course_info: "英語I".to_string(),
        module: "お知らせ".to_string(),
        content: content.to_string(),
        url: String::new(),
        idnumber: String::new(),
    }
}

#[test]
fn cache_due_respects_age_unless_forced() {
    assert!(cache_refresh_due(None, 1_000, 300, false));
    assert!(!cache_refresh_due(Some(900), 1_000, 300, false));
    assert!(cache_refresh_due(Some(900), 1_000, 300, true));
    assert!(cache_refresh_due(Some(700), 1_000, 300, false));
}

#[test]
fn strips_trailing_timestamp() {
    assert_eq!(
        strip_trailing_luna_timestamp("第5回レポートについて (2025/07/01 12:00)"),
        "第5回レポートについて"
    );
    assert_eq!(
        strip_trailing_luna_timestamp("課題（第1回）(2025/07/01 12:00)"),
        "課題（第1回）"
    );
}

#[test]
fn keeps_non_timestamp_parens() {
    assert_eq!(
        strip_trailing_luna_timestamp("レポート（第1回）"),
        "レポート（第1回）"
    );
    assert_eq!(
        strip_trailing_luna_timestamp("small (note)"),
        "small (note)"
    );
}

#[test]
fn revision_key_stable_across_timestamp_change() {
    // Same post, only the displayed timestamp changed → same revision key,
    // so it must not re-fire as an update.
    let a = notif("・お知らせ が追加されました。(2025/07/01 12:00)");
    let b = notif("・お知らせ が追加されました。(2025/07/02 09:30)");
    assert_eq!(luna_revision_key(&a), luna_revision_key(&b));
}

#[test]
fn revision_key_differs_on_genuine_update() {
    let added = notif("・お知らせ が追加されました。(2025/07/01 12:00)");
    let updated = notif("・お知らせ が更新されました。(2025/07/01 12:00)");
    assert_ne!(luna_revision_key(&added), luna_revision_key(&updated));
}
