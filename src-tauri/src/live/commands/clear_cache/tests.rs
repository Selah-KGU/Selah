use super::*;
use crate::live::{cache, tests::transcript::recording, LiveFinishPhase};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{
    test::{mock_builder, mock_context, noop_assets},
    Manager,
};

fn course() -> LiveCourseInfo {
    recording().course
}

#[tokio::test(flavor = "current_thread")]
async fn validation_skips_storage_and_only_non_free_blank_names_fail() {
    let state = LiveState::new();
    let mut free = course();
    free.is_free_note = true;
    free.course_name = String::new();
    clear(state.clone(), free, |_| {
        panic!("free note must skip storage")
    })
    .await
    .unwrap();
    let mut blank = course();
    blank.course_name = " \t\n ".into();
    assert_eq!(
        clear(state, blank, |_| panic!("blank name must skip storage"))
            .await
            .unwrap_err(),
        "講義名が空です"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn recording_and_finish_owners_are_protected_and_other_courses_release_the_live_lock_before_io(
) {
    for phase in [None, Some(LiveFinishPhase::SavingRecord)] {
        let state = LiveState::new();
        let mut session = recording();
        session.finish_phase = phase;
        *state.session.lock().unwrap() = Some(session);
        assert_eq!(
            clear(state.clone(), course(), |_| panic!(
                "active course must not touch storage"
            ))
            .await
            .unwrap_err(),
            "録音中または保存中の講義データは削除できません"
        );
        let mut another = course();
        another.course_name = "different complete course 🌙".into();
        let checked = state.clone();
        clear(state.clone(), another, move |course| {
            assert_eq!(course.course_name, "different complete course 🌙");
            assert!(
                checked.session.try_lock().is_ok(),
                "IO holds the LIVE owner"
            );
            assert!(
                checked.persistence.gate.try_lock().is_err(),
                "IO lost storage ownership"
            );
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            state.session.lock().unwrap().as_ref().unwrap().session_id,
            "recording-test"
        );
    }
    let state = LiveState::new();
    let mut free = recording();
    free.course.is_free_note = true;
    *state.session.lock().unwrap() = Some(free);
    clear(state, course(), |_| Ok(())).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn waiting_for_storage_does_not_block_async_work_and_rechecks_a_recording_admitted_while_waiting(
) {
    let state = LiveState::new();
    let holding = state.clone();
    let (locked, entered) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _storage = holding.persistence.gate.lock().unwrap();
        locked.send(()).unwrap();
        released
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    });
    entered.await.unwrap();
    let clearing = tokio::spawn(clear(state.clone(), course(), |_| {
        panic!("replacement recording must protect cache")
    }));
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    let pending = !clearing.is_finished();
    *state.session.lock().unwrap() = Some(recording());
    release.send(()).unwrap();
    holder.join().unwrap();
    assert!(pending, "clear ignored its storage gate");
    assert_eq!(
        clearing.await.unwrap().unwrap_err(),
        "録音中または保存中の講義データは削除できません"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn current_io_errors_reach_callers_and_panics_and_poisoned_owners_are_not_successful_clears()
{
    let state = LiveState::new();
    assert_eq!(
        clear(state.clone(), course(), |_| cache::denied_fixture())
            .await
            .unwrap_err(),
        "LIVEキャッシュの削除失敗: fixture deletion denied"
    );
    clear(state.clone(), course(), |_| Ok(())).await.unwrap();
    assert!(
        clear(state.clone(), course(), |_| panic!("fixture removal panic"))
            .await
            .unwrap_err()
            .starts_with("Liveキャッシュ削除処理失敗:")
    );
    // Worker unwind poisons storage. Existing recovery of that gate allows retry.
    clear(state.clone(), course(), |_| Ok(())).await.unwrap();
    let poisoned = state.clone();
    let _ = std::thread::spawn(move || {
        let _owner = poisoned.session.lock().unwrap();
        panic!("fixture owner panic");
    })
    .join();
    assert_eq!(
        clear(state, course(), |_| panic!("poison must stop IO"))
            .await
            .unwrap_err(),
        "Live state lock failed"
    );
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("selah-clear-wire-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn base(&self) -> PathBuf {
        self.0.join("cache.json")
    }
    fn journal(&self) -> PathBuf {
        self.0.join("lines.ndjson")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[derive(Clone)]
struct RemovalFixture {
    directory: Arc<Directory>,
    denied: Arc<AtomicBool>,
}

// Same domain worker and file-removal implementation as the actual command;
// only the path resolver and filesystem denial are isolated test boundaries.
#[tauri::command]
async fn clear_fixture(
    state: tauri::State<'_, LiveState>,
    fixture: tauri::State<'_, RemovalFixture>,
    course: LiveCourseInfo,
) -> Result<(), String> {
    let fixture = fixture.inner().clone();
    clear(state.inner().clone(), course, move |_| {
        if fixture.denied.load(Ordering::Acquire) {
            cache::denied_fixture()
        } else {
            cache::remove_day_cache_files(&fixture.directory.base(), &fixture.directory.journal())
        }
    })
    .await
}

#[test]
fn native_ipc_preserves_error_strings_and_success_null_and_retries_actual_temporary_files() {
    let fixture = RemovalFixture {
        directory: Arc::new(Directory::new()),
        denied: Arc::new(AtomicBool::new(true)),
    };
    std::fs::write(fixture.directory.base(), b"base transcript").unwrap();
    std::fs::write(fixture.directory.journal(), b"new transcript").unwrap();
    let app = mock_builder()
        .manage(LiveState::new())
        .manage(fixture.clone())
        .invoke_handler(tauri::generate_handler![clear_fixture])
        .build(mock_context(noop_assets()))
        .unwrap();
    let window =
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
            .build()
            .unwrap();
    let call = |course: LiveCourseInfo| {
        tauri::test::get_ipc_response(
            &window,
            tauri::webview::InvokeRequest {
                cmd: "clear_fixture".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(json!({"course":course})),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            },
        )
    };
    let error = call(course()).unwrap_err();
    assert_eq!(
        std::fs::read(fixture.directory.base()).unwrap(),
        b"base transcript"
    );
    assert_eq!(
        std::fs::read(fixture.directory.journal()).unwrap(),
        b"new transcript"
    );
    fixture.denied.store(false, Ordering::Release);
    let success = call(course()).unwrap().deserialize::<Value>().unwrap();
    assert!(!fixture.directory.base().exists());
    assert!(!fixture.directory.journal().exists());
    let repeated = call(course()).unwrap().deserialize::<Value>().unwrap();
    let wire = json!({"error":error,"success":success,"repeated":repeated});
    if let Ok(path) = std::env::var("SELAH_CLEAR_CACHE_WIRE") {
        std::fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&wire).unwrap()),
        )
        .unwrap();
    }
    assert_eq!(
        wire,
        serde_json::from_str::<Value>(include_str!(
            "../../../../../tests/fixtures/live-clear-cache-wire.json"
        ))
        .unwrap()
    );
    assert!(app.state::<LiveState>().session.lock().unwrap().is_none());
}
