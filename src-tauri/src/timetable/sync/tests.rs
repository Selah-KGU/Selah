use super::*;
use crate::db::Database;

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn open() -> (Database, Self) {
        let directory =
            Self(std::env::temp_dir().join(format!("selah-primary-{}", uuid::Uuid::new_v4())));
        (Database::open(&directory.0).unwrap(), directory)
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn luna_week_labels_exist_without_kgc_and_advance_from_the_calendar() {
    let monday = chrono::NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
    let (current, next) = crate::academic_period::timetable_week_labels("", "", monday);
    assert_eq!(current, "2026年10月19日～2026年10月24日");
    assert_eq!(next, "2026年10月26日～2026年10月31日");
    let (_, advanced) = crate::academic_period::timetable_week_labels(
        &current,
        &next,
        monday + chrono::Duration::days(7),
    );
    assert_eq!(advanced, "2026年11月02日～2026年11月07日");
    let (current, _) =
        crate::academic_period::timetable_week_labels("2026/10/19～2026/10/24", "", monday);
    assert_eq!(current, "2026/10/19～2026/10/24");
}

#[test]
fn saved_kgc_warnings_do_not_reappear_when_loading_luna_courses() {
    let (db, _directory) = Temporary::open();
    db.replace_luna_courses(&[crate::luna_parser::LunaCourse {
        idnumber: "local-course".into(),
        name: "Luna course".into(),
        teacher: "Teacher".into(),
        day: 1,
        period: 1,
    }])
    .unwrap();
    db.save_data_cache(
        "schedule_kgc_warning",
        "KGC に接続できません。Luna と保存済みの時間割を表示しています。",
    )
    .unwrap();
    for _ in 0..2 {
        let response = super::super::snapshot::build_schedule_snapshot_from_db(&db).unwrap();
        assert!(response.kgc_warning.is_empty());
        assert_eq!(response.raw.luna_courses[0].name, "Luna course");
    }
}

#[tokio::test]
async fn an_unavailable_luna_session_does_not_change_the_saved_snapshot() {
    let (db, _directory) = Temporary::open();
    db.save_snapshot_state(&SnapshotState {
        luna_year: "2000".into(),
        luna_term: "02".into(),
        ..Default::default()
    })
    .unwrap();
    let before = serde_json::to_value(db.get_snapshot_state().unwrap().unwrap()).unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        refresh_primary_schedule(&LunaState, &db),
    )
    .await
    .expect("primary refresh must not wait for KGC")
    .unwrap_err();
    assert_eq!(error, crate::luna_client::LUNA_AUTH_REQUIRED_MSG);
    assert_eq!(
        serde_json::to_value(db.get_snapshot_state().unwrap().unwrap()).unwrap(),
        before
    );
}

#[tokio::test]
async fn primary_http_refresh_validates_pages_and_persists_only_real_timetables() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (db, _directory) = Temporary::open();
    let saturday = chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
    let target = crate::academic_period::calendar_academic_period(saturday);
    let (year, term) = target
        .map(|p| (p.year, p.term))
        .unwrap_or(("2026".into(), "03".into()));
    db.save_snapshot_state(&SnapshotState {
        current_week_label: "2026/10/12～2026/10/17".into(),
        next_week_label: "2026/10/19～2026/10/24".into(),
        luna_year: year.clone(),
        luna_term: term.clone(),
        ..Default::default()
    })
    .unwrap();
    let page = |course: &str| {
        format!(
            r#"
        <select id="nendo"><option value="{year}" selected>{year}</option></select>
        <select id="kikanCd"><option value="{term}" selected>{term}</option></select>
        <div class="div-table-data-row"><div class="div-table-colomn-period">１時限</div>
          <div class="div-table-cell">{course}</div><div class="div-table-cell"></div>
          <div class="div-table-cell"></div><div class="div-table-cell"></div>
          <div class="div-table-cell"></div><div class="div-table-cell"></div></div>"#
        )
    };
    let course = format!(
        r#"<button class="timetable-course-top-btn" id="{year}00000000{term}01">Fresh Luna</button>"#
    );
    let pages = [
        page(&course),
        "<html>Maintenance</html>".into(),
        r#"<div class="login-body">linkCommonLogin</div>"#.into(),
        page(""),
    ];
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for page in pages {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 8192];
            let read = connection.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            assert!(
                request.starts_with("GET /lms/timetable HTTP/1.1"),
                "{request}"
            );
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", page.len(), page);
            connection.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    // The real coordinator's KGC recovery lock remains occupied throughout.
    let _kgc_recovery = crate::session_coordinator::SESSIONS
        .lock_service(crate::session_coordinator::Service::Kgc)
        .await;
    let fresh = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        refresh_primary_schedule_with_http(&http, &db, &base, saturday),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(fresh.raw.luna_courses[0].name, "Fresh Luna");
    assert!(fresh.raw.kgc_entries_current.is_empty());
    assert_eq!(fresh.raw.current_week_label, "2026/10/19～2026/10/24");
    assert_eq!(fresh.raw.next_week_label, "2026年10月26日～2026年10月31日");
    let saved = db.get_snapshot_state().unwrap().unwrap();
    assert_eq!(fresh.snapshot_updated_at, saved.updated_at);
    assert!(!saved.current_week_label.is_empty());
    for auth_error in [false, true] {
        let error = refresh_primary_schedule_with_http(&http, &db, &base, saturday)
            .await
            .unwrap_err();
        assert_eq!(
            error == crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
            auth_error
        );
        assert_eq!(db.get_luna_courses().unwrap()[0].name, "Fresh Luna");
        assert_eq!(
            serde_json::to_value(db.get_snapshot_state().unwrap().unwrap()).unwrap(),
            serde_json::to_value(&saved).unwrap()
        );
    }
    let empty = refresh_primary_schedule_with_http(&http, &db, &base, saturday)
        .await
        .unwrap();
    assert!(empty.raw.luna_courses.is_empty());
    assert!(db.get_luna_courses().unwrap().is_empty());
    server.await.unwrap();
}
