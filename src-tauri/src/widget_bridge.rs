//! macOS WidgetKit snapshot publisher.
//!
//! The extension reads \`widget-snapshot.json\`. A bundled Selah.app is the widget's
//! parent. \`tauri dev\` is a bare binary, so a small host app keeps the extension
//! registered and reloads timelines when the snapshot changes.
//!
//! The host must not reuse \`com.kgu.selah.widget\`. WidgetKit stores timelines in the
//! extension container and rejects an archive whose parent bundle cannot be looked up
//! (\`bundleStubNotSupported\`). A second copy then leaves the desktop widget blank.

use chrono::Local;

use crate::db::Database;

#[path = "widget_bridge/host.rs"]
mod host;
#[path = "widget_bridge/model.rs"]
mod model;
#[path = "widget_bridge/snapshot.rs"]
mod snapshot;

#[allow(unused_imports)]
pub use model::{WidgetClass, WidgetSnapshot, WidgetTodo};

const SNAPSHOT_NAME: &str = "widget-snapshot.json";
const HOST_APP_NAME: &str = "Selah Widget.app";
const WIDGET_BUNDLE_ID: &str = "com.kgu.selah.widget";
const DEV_WIDGET_BUNDLE_ID: &str = "com.kgu.selah.dev.widget";

#[cfg(target_os = "macos")]
use host::{install_host, retire_dev_host, running_inside_widget_app};
use host::{macos_major, reload_timelines, write_snapshot};
use snapshot::snapshot_from_db;

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
        if macos_major() < 14 {
            return;
        }
        if running_inside_widget_app() {
            if let Err(error) = retire_dev_host() {
                log::warn!("widget dev host cleanup failed: {error}");
            }
            return;
        }
        if let Err(error) = install_host() {
            log::warn!("widget host install failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Weekday};

    use super::host::host_extension_plist;
    use super::model::{ClassSource, TodoSource};
    use super::snapshot::{
        assemble_snapshot, classes_from_sources, parse_deadline, todos_from_sources,
    };

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
    fn dev_host_extension_does_not_reuse_release_bundle_id() {
        let plist = format!("<key>CFBundleIdentifier</key><string>{WIDGET_BUNDLE_ID}</string>");
        let updated = host_extension_plist(&plist);
        assert!(updated.contains(DEV_WIDGET_BUNDLE_ID));
        assert!(!updated.contains(&format!("<string>{WIDGET_BUNDLE_ID}</string>")));
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
