//! Tauri commands for the LIVE session.
//!
//! Handlers translate UI actions into session changes. Summary generation and
//! the flush driver stay beside the session state.

use super::*;
use serde::Serialize;
mod clear_cache;
mod tray_status;
#[cfg(test)]
use tray_status::snapshot as tray_status_snapshot;

#[tauri::command]
pub async fn live_get_session(
    state: tauri::State<'_, LiveState>,
) -> Result<tauri::ipc::Response, String> {
    response::current(state.inner().clone()).await
}

/// Startup only decides whether to reopen LIVE; no transcript/summary snapshot
/// or microphone state is needed. Wait for a busy owner on a blocking worker.
#[tauri::command]
pub async fn live_has_active_session(
    state: tauri::State<'_, LiveState>,
) -> Result<tauri::ipc::Response, String> {
    active_session_reply(state.inner().clone()).await
}

async fn active_session_reply(state: LiveState) -> Result<tauri::ipc::Response, String> {
    response::work("Live状態取得処理失敗", move || {
        state
            .session
            .lock()
            .map(|session| session.is_some())
            .map_err(|_| "Live state lock failed".to_string())
    })
    .await
}

#[tauri::command]
pub async fn live_get_surface(
    state: tauri::State<'_, LiveState>,
) -> Result<tauri::ipc::Response, String> {
    let state = state.inner().clone();
    response::work("Live状態取得処理失敗", move || {
        Ok(surface::current(&state))
    })
    .await
}

#[tauri::command]
pub async fn live_get_surface_compact(
    state: tauri::State<'_, LiveState>,
) -> Result<tauri::ipc::Response, String> {
    let state = state.inner().clone();
    response::work("Live状態取得処理失敗", move || {
        Ok(surface::CompactSurfaceSnapshot::from(surface::current(
            &state,
        )))
    })
    .await
}

#[tauri::command]
pub async fn live_get_tray_status(
    state: tauri::State<'_, LiveState>,
) -> Result<tauri::ipc::Response, String> {
    tray_status::reply(state.inner().clone(), crate::stt::stt_get_stream_state).await
}

/// Peek at the day cache for a course without starting a session.
/// Returns an inactive snapshot with the cached transcript/summaries, or empty if no cache.
#[tauri::command]
pub async fn live_peek_day_cache(
    state: tauri::State<'_, LiveState>,
    course: LiveCourseInfo,
) -> Result<tauri::ipc::Response, String> {
    peek_reply(
        state.inner().clone(),
        course,
        std::convert::identity::<LiveSessionSnapshot>,
    )
    .await
}

#[tauri::command]
pub async fn live_peek_day_surface(
    state: tauri::State<'_, LiveState>,
    course: LiveCourseInfo,
) -> Result<tauri::ipc::Response, String> {
    peek_reply(
        state.inner().clone(),
        course,
        surface::LiveSurfaceSnapshot::from,
    )
    .await
}

#[tauri::command]
pub async fn live_peek_day_surface_compact(
    state: tauri::State<'_, LiveState>,
    course: LiveCourseInfo,
) -> Result<tauri::ipc::Response, String> {
    peek_reply(
        state.inner().clone(),
        course,
        surface::CompactSurfaceSnapshot::from,
    )
    .await
}

async fn peek_reply<T: Serialize + Send + 'static>(
    state: LiveState,
    course: LiveCourseInfo,
    project: fn(LiveSessionSnapshot) -> T,
) -> Result<tauri::ipc::Response, String> {
    response::work("Liveキャッシュの読み込み失敗", move || {
        Ok(project(peek_snapshot(&state, course, load_day_cache)))
    })
    .await
}

pub(super) fn peek_snapshot(
    state: &LiveState,
    course: LiveCourseInfo,
    load: impl FnOnce(&LiveCourseInfo) -> Option<cache::LiveDayCache>,
) -> LiveSessionSnapshot {
    let _storage = state
        .persistence
        .gate
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    match load(&course) {
        Some(cache) => LiveSessionSnapshot {
            update_revision: 0,
            session_id: None,
            active: false,
            course: Some(course),
            started_at: Some(cache.started_at),
            transcript_lines: Arc::new(cache.transcript_lines),
            pending_lines: Arc::new(Vec::new()),
            summaries: Arc::new(cache.summaries),
            next_summary_at_ms: None,
            summarizing: false,
            finish_phase: None,
            finish_revision: 0,
        },
        None => empty_snapshot(),
    }
}

#[tauri::command]
pub async fn live_start_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    course: LiveCourseInfo,
) -> Result<tauri::ipc::Response, String> {
    start_reply(
        app,
        state.inner().clone(),
        course,
        std::convert::identity::<LiveSessionSnapshot>,
    )
    .await
}

#[tauri::command]
pub async fn live_start_surface(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    course: LiveCourseInfo,
) -> Result<tauri::ipc::Response, String> {
    start_reply(
        app,
        state.inner().clone(),
        course,
        surface::LiveSurfaceSnapshot::from,
    )
    .await
}

#[tauri::command]
pub async fn live_start_surface_compact(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    course: LiveCourseInfo,
) -> Result<tauri::ipc::Response, String> {
    start_reply(
        app,
        state.inner().clone(),
        course,
        surface::CompactSurfaceSnapshot::from,
    )
    .await
}

async fn start_reply<T: Serialize + Send + 'static>(
    app: tauri::AppHandle,
    state: LiveState,
    course: LiveCourseInfo,
    project: fn(LiveSessionSnapshot) -> T,
) -> Result<tauri::ipc::Response, String> {
    response::work("Live開始処理失敗", move || {
        crate::ai::refresh_live_summary_interval();
        let snapshot = start_session(&state, course)?;
        let id = snapshot.session_id.clone();
        let result = project(snapshot);
        // Guards and surface replies' full indexes are released before a
        // notification can trigger more speech, and before JSON encoding.
        emit_live_update(&app, &state);
        if let Some(id) = id {
            start_live_flush_driver(app, id);
        }
        Ok(result)
    })
    .await
}

fn start_session(
    state: &LiveState,
    mut course: LiveCourseInfo,
) -> Result<LiveSessionSnapshot, String> {
    let _storage = state
        .persistence
        .gate
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    if crate::app_shutdown::is_shutting_down() {
        return Err("アプリケーションを終了中です".into());
    }

    if state
        .session
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?
        .is_some()
    {
        return Err(
            "Liveセッションが使用中です。停止・保存してから新しい録音を開始してください".into(),
        );
    }
    if !course.is_free_note && course.course_name.trim().is_empty() {
        return Err("講義名が空です".into());
    }
    if course.is_free_note {
        course.course_name = FREE_NOTE_FOLDER_NAME.to_string();
        course.course_code.clear();
        course.room.clear();
        course.teacher.clear();
        course.day = 0;
        course.period = 0;
        course.time_label.clear();
    } else {
        course.course_name = course.course_name.trim().to_string();
        course.course_code = course.course_code.trim().to_string();
        course.room = course.room.trim().to_string();
        course.teacher = course.teacher.trim().to_string();
        course.time_label = course.time_label.trim().to_string();
    }

    let now = Local::now();

    // Load accumulated data from earlier in the same course today
    let cached = load_day_cache(&course);
    let is_fresh_start = cached.is_none();
    let (prev_transcript, prev_summaries, original_start) = match cached {
        Some(cache) => (cache.transcript_lines, cache.summaries, cache.started_at),
        None => (Vec::new(), Vec::new(), format_datetime(now)),
    };
    let started_at = chrono::NaiveDateTime::parse_from_str(&original_start, "%Y-%m-%d %H:%M:%S")
        .map(|naive| naive.and_local_timezone(Local).unwrap())
        .unwrap_or(now);
    let batch_started_at = latest_summary_end_datetime(started_at, &prev_summaries)
        .or_else(|| {
            if prev_summaries.is_empty() {
                None
            } else {
                Some(last_transcript_line_datetime(
                    started_at,
                    &prev_transcript,
                    now,
                ))
            }
        })
        .unwrap_or(now);

    let cache_progress = CacheProgress::restored(prev_transcript.len(), prev_summaries.len());
    let session_id = uuid::Uuid::new_v4().to_string();
    let session = LiveSession {
        session_id: session_id.clone(),
        course,
        started_at,
        transcript_lines: Arc::new(prev_transcript),
        pending_lines: Arc::new(Vec::new()),
        summaries: Arc::new(prev_summaries),
        batch_started_at,
        flush_in_flight: false,
        is_fresh_start,
        cache_progress,
        finish_phase: None,
        finish_revision: 0,
    };
    let mut guard = state
        .session
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    if guard.is_some() {
        return Err(
            "Liveセッションが使用中です。停止・保存してから新しい録音を開始してください".into(),
        );
    }
    *guard = Some(session);
    let snapshot = state.capture_snapshot(guard.as_ref());
    drop(guard);
    Ok(snapshot)
}

pub(crate) fn append_recognized_transcript(
    app: &tauri::AppHandle,
    text: &str,
    expected_session_id: &str,
    seq: u64,
) -> Result<bool, String> {
    append_transcript(
        app,
        app.state::<LiveState>().inner(),
        text,
        Some(expected_session_id),
        Some(seq),
    )
}

pub(super) fn append_transcript(
    app: &tauri::AppHandle,
    state: &LiveState,
    text: &str,
    expected: Option<&str>,
    seq: Option<u64>,
) -> Result<bool, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(false);
    }
    let line = LiveTranscriptLine {
        text: text.to_string(),
        at: Local::now().format("%H:%M:%S").to_string(),
    };
    // Validate ownership under the same lock that appends the line. A final
    // from a canceled recording must never enter the next course's session.
    let Some(mut update) = state.append_line_for_session(expected, line)? else {
        return Ok(false);
    };
    update.seq = seq;
    auto_save_day_cache(state, false);
    state.notify_flush_driver();
    // One committed delta serves the page and native overlays. Its owner and
    // capture order let overlays reject delayed finals without losing storage.
    let _ = app.emit("live-transcript-appended", update);
    Ok(true)
}

#[tauri::command]
pub async fn live_flush_summary(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    force: bool,
) -> Result<tauri::ipc::Response, String> {
    let snapshot = live_flush_summary_with_side_effects(&app, state.inner(), force).await?;
    response::encode(snapshot).await
}

#[tauri::command]
pub async fn live_generate_overall_summary(
    state: tauri::State<'_, LiveState>,
    session_id: String,
) -> Result<tauri::ipc::Response, String> {
    // The overall summary is a quick "so far" snapshot: it reads the existing
    // summary chunks plus the tail of the live transcript (which already holds
    // every appended line, pending ones included). It must NOT force a chunk
    // flush first — that ran the full per-chunk pipeline (summary + terms +
    // whiteboard generation, several AI calls) and could block up to
    // LIVE_FLUSH_FORCE_WAIT_ATTEMPTS while a scheduled flush was in flight,
    // which is what made this button take abnormally long.
    let (course, started_at, transcript_lines, summaries) =
        overall_summary_input(&state, &session_id)?;

    let summary = generate_overall_summary(
        &course,
        started_at,
        Local::now(),
        &summaries,
        &transcript_lines,
    )
    .await;
    response::encode(summary).await
}

type OverallSummaryInput = (
    LiveCourseInfo,
    DateTime<Local>,
    LiveTranscriptLines,
    LiveSummaryChunks,
);

fn overall_summary_input(state: &LiveState, expected: &str) -> Result<OverallSummaryInput, String> {
    let guard = state
        .session
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    let session = guard
        .as_ref()
        .filter(|session| session.session_id == expected)
        .ok_or_else(|| "Liveセッションが切り替わりました".to_string())?;
    if session.finish_phase.is_some() {
        return Err("Liveセッションを保存中です".into());
    }
    if session.transcript_lines.is_empty() {
        return Err("全体要約を生成できる文字起こしがまだありません".to_string());
    }
    Ok((
        session.course.clone(),
        session.started_at,
        session.transcript_lines.clone(),
        session.summaries.clone(),
    ))
}

#[tauri::command]
pub async fn live_cancel_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    session_id: String,
) -> Result<(), String> {
    let storage_state = state.inner().clone();
    tokio::task::spawn_blocking(move || cancel_session(&storage_state, &session_id))
        .await
        .map_err(|error| format!("Live破棄処理失敗: {error}"))??;
    emit_live_update(&app, state.inner());
    Ok(())
}

fn cancel_session(state: &LiveState, expected: &str) -> Result<(), String> {
    let _storage = state
        .persistence
        .gate
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let mut guard = state
        .session
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    if guard
        .as_ref()
        .is_none_or(|session| session.session_id != expected)
    {
        return Err("Liveセッションが切り替わりました".into());
    }
    // Grab info we need to scrub on-disk artifacts before dropping the session.
    // The flush path may have written a partial .md (and recorded it in the
    // downloads history) — leaving those behind would contradict the UI's
    // "破棄" message. But only scrub when this session was a fresh start;
    // a resumed session shares its .md and day_cache with earlier completed
    // recordings today, and we must not destroy that prior content.
    let cleanup = guard.as_ref().map(|s| {
        (
            s.course.clone(),
            s.started_at,
            !s.transcript_lines.is_empty(),
            s.is_fresh_start,
        )
    });
    if guard
        .as_ref()
        .is_some_and(|session| session.finish_phase.is_some())
    {
        return Err("Liveセッションを保存中です。完了後にもう一度操作してください".into());
    }
    *guard = None;
    drop(guard);
    state.persistence.wake();

    if let Some((course, started_at, had_transcript, is_fresh_start)) = cleanup {
        if is_fresh_start {
            if had_transcript {
                let partial_path =
                    live_storage_dir(&course).join(formal_markdown_filename(&course, started_at));
                if partial_path.exists() {
                    let _ = std::fs::remove_file(&partial_path);
                }
                crate::commands::remove_download_records_by_path(&partial_path.to_string_lossy());
            }
            if !course.is_free_note {
                if let Err(error) = remove_day_cache(&course) {
                    log::warn!("[Live] cancelled recording cache cleanup failed: {error}");
                }
            }
        }
    }

    state.notify_flush_driver();
    Ok(())
}

/// Clear the day cache for a specific course, removing all accumulated transcript/summary data.
#[tauri::command]
pub async fn live_clear_day_cache(
    state: tauri::State<'_, LiveState>,
    course: LiveCourseInfo,
) -> Result<(), String> {
    clear_cache::clear(state.inner().clone(), course, remove_day_cache).await
}

#[tauri::command]
pub async fn live_finish_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    session_id: String,
) -> Result<tauri::ipc::Response, String> {
    finish_reply(app, state.inner(), session_id, response::FinishReply::Full).await
}

#[tauri::command]
pub async fn live_finish_surface(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    session_id: String,
) -> Result<tauri::ipc::Response, String> {
    finish_reply(
        app,
        state.inner(),
        session_id,
        response::FinishReply::Surface,
    )
    .await
}

#[tauri::command]
pub async fn live_finish_surface_compact(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    session_id: String,
) -> Result<tauri::ipc::Response, String> {
    finish_reply(
        app,
        state.inner(),
        session_id,
        response::FinishReply::CompactSurface,
    )
    .await
}

async fn finish_reply(
    app: tauri::AppHandle,
    state: &LiveState,
    session_id: String,
    reply: response::FinishReply,
) -> Result<tauri::ipc::Response, String> {
    let ownership = state.begin_finish(&session_id)?;
    emit_live_update(&app, state);
    let result = finish_session(&app, state, &ownership.session_id, reply).await;
    drop(ownership);
    if result.is_err() {
        emit_live_update(&app, state);
    }
    result
}

async fn finish_session(
    app: &tauri::AppHandle,
    state: &LiveState,
    expected: &str,
    reply: response::FinishReply,
) -> Result<tauri::ipc::Response, String> {
    // Final decoding can outlive the microphone stop request. The backend
    // waits too, so direct callers and a reloaded UI cannot save a partial tail.
    crate::stt::stop_stream_for_caller(Some("live".into()), Some(expected.into())).await?;
    emit_live_finish_progress(app, state, expected, LiveFinishPhase::SavingRecord)?;
    let is_empty = {
        let guard = state
            .session
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .filter(|session| session.session_id == expected)
            .ok_or_else(|| "Liveセッションが切り替わりました".to_string())?;
        session.transcript_lines.is_empty()
    };
    if is_empty {
        let storage_state = state.clone();
        let expected = expected.to_string();
        let snapshot = tokio::task::spawn_blocking(move || {
            let _storage = storage_state
                .persistence
                .gate
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut guard = storage_state
                .session
                .lock()
                .map_err(|_| "Live state lock failed".to_string())?;
            let session = guard
                .as_ref()
                .filter(|session| {
                    session.session_id == expected && session.transcript_lines.is_empty()
                })
                .ok_or_else(|| "Liveセッションが切り替わりました".to_string())?;
            let snapshot = storage_state.capture_completed_snapshot(session);
            let course = session.course.clone();
            *guard = None;
            drop(guard);
            if !course.is_free_note {
                if let Err(error) = remove_day_cache(&course) {
                    log::warn!("[Live] empty recording cache cleanup failed: {error}");
                }
            }
            Ok::<_, String>(snapshot)
        })
        .await
        .map_err(|error| format!("Live保存処理失敗: {error}"))??;
        state.notify_flush_driver();
        emit_live_update(app, state);
        return saved_reply(
            app,
            LiveSaveResult {
                saved: false,
                path: String::new(),
                markdown: String::new(),
                snapshot,
                suggested_todos: Vec::new(),
                todos_pending: false,
            },
            reply,
        )
        .await;
    }
    let (started_at, pending_line_count) = {
        let guard = state
            .session
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let Some(session) = guard.as_ref() else {
            return Err("Liveセッションが開始されていません".to_string());
        };
        if session.session_id != expected {
            return Err("Liveセッションが切り替わりました".into());
        }

        (session.started_at, session.pending_lines.len())
    };

    let pre_ai_saved_at = Local::now();
    persist_finish_files(
        state,
        expected,
        PRE_AI_OVERALL_SUMMARY.to_string(),
        pre_ai_saved_at,
        false,
    )
    .await?;

    let ai_config = tokio::task::spawn_blocking(crate::ai::load_ai_config)
        .await
        .map_err(|error| format!("AI設定読み込み失敗: {error}"))?;
    let will_run_finish_ai = should_run_finish_ai(&ai_config.provider, started_at, pre_ai_saved_at);
    let needs_final_chunk_ai =
        should_require_finish_chunk_ai(started_at, pre_ai_saved_at, pending_line_count);
    if needs_final_chunk_ai || will_run_finish_ai {
        emit_live_finish_progress(app, state, expected, LiveFinishPhase::Summarizing)?;
    }
    if needs_final_chunk_ai {
        // Retry the closing segment a few times before giving up. If it still
        // fails, degrade to saving the transcript plus the segments we already
        // have rather than failing the whole stop and losing everything — the
        // markdown build below works from whatever summaries exist.
        if let Err(err) = flush_final_summary_with_retry(&state).await {
            log::warn!(
                "[Live] final chunk summary gave up after retries: {err}; \
                 saving transcript without the last segment"
            );
        }
    } else {
        // Non-fatal for short sessions: they intentionally skip AI and save the transcript as-is.
        let _ = flush_session_summary(&state, true).await;
    }

    let (course, started_at, transcript_lines, summaries) = {
        let guard = state
            .session
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .filter(|session| session.session_id == expected)
            .ok_or_else(|| "Liveセッションが切り替わりました".to_string())?;
        (
            session.course.clone(),
            session.started_at,
            session.transcript_lines.clone(),
            session.summaries.clone(),
        )
    };

    let ended_at = Local::now();
    let should_run_finish_ai = should_run_finish_ai(&ai_config.provider, started_at, ended_at);
    let overall_summary =
        generate_overall_summary(&course, started_at, ended_at, &summaries, &transcript_lines)
            .await;
    // TODO/DDL judgment is the slowest AI step. It must NOT block saving — we
    // run it in the background (after the file is written and the session is
    // cleared) and push the result through the `live-todo-suggestions` event so
    // the UI can jump straight to the TODO page and add them there.
    let run_todos_in_background = !course.is_free_note
        && !transcript_lines.is_empty()
        && !should_skip_ai_summarization(started_at, ended_at)
        && should_run_finish_ai;

    emit_live_finish_progress(app, state, expected, LiveFinishPhase::SavingFinal)?;
    let (path, markdown, snapshot) =
        persist_finish_files(state, expected, overall_summary, ended_at, true).await?;
    let path_str = path.to_string_lossy().to_string();
    state.notify_flush_driver();

    // Kick off TODO/DDL judgment without blocking the return. Suggestions arrive
    // via `live-todo-suggestions`; the frontend moves to the TODO page meanwhile.
    if run_todos_in_background {
        let app_bg = app.clone();
        let course_bg = course.clone();
        let summaries_bg = summaries.clone();
        let transcript_bg = transcript_lines.clone();
        let source_path = path_str.clone();
        tauri::async_runtime::spawn(async move {
            let suggestions = extract_todo_suggestions(
                &app_bg,
                &course_bg,
                &summaries_bg,
                &transcript_bg,
                ended_at,
            )
            .await;
            let payload = LiveTodoSuggestionsEvent {
                suggestions,
                source_path,
            };
            let _ = app_bg.emit("live-todo-suggestions", &payload);
        });
    }

    // Only background TODO work still needs the complete history. Do not keep
    // this finisher's earlier AI capture alive across page reply serialization.
    drop(transcript_lines);
    drop(summaries);
    let result = LiveSaveResult {
        saved: true,
        path: path_str.clone(),
        markdown,
        snapshot,
        suggested_todos: Vec::new(),
        todos_pending: run_todos_in_background,
    };
    let response = saved_reply(app, result, reply).await;
    emit_live_update(app, state);
    response
}

async fn saved_reply(
    app: &tauri::AppHandle,
    result: LiveSaveResult,
    reply: response::FinishReply,
) -> Result<tauri::ipc::Response, String> {
    let event_app = app.clone();
    response::saved(result, reply, move |name, json| {
        let _ = event_app.emit_str(name, json);
    })
    .await
}

async fn persist_finish_files(
    state: &LiveState,
    expected: &str,
    overall_summary: String,
    ended_at: chrono::DateTime<Local>,
    clear: bool,
) -> Result<(std::path::PathBuf, String, LiveSessionSnapshot), String> {
    let state = state.clone();
    let expected = expected.to_string();
    tokio::task::spawn_blocking(move || {
        let _storage = state
            .persistence
            .gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            let mut snapshot = {
                let guard = state
                    .session
                    .lock()
                    .map_err(|_| "Live state lock failed".to_string())?;
                let session = guard
                    .as_ref()
                    .filter(|session| {
                        session.session_id == expected && session.finish_phase.is_some()
                    })
                    .ok_or_else(|| "Liveセッションが切り替わりました".to_string())?;
                state.capture_snapshot(Some(session))
            };
            let course = snapshot
                .course
                .as_ref()
                .ok_or_else(|| "Live講義情報がありません".to_string())?;
            let started_at = {
                let guard = state
                    .session
                    .lock()
                    .map_err(|_| "Live state lock failed".to_string())?;
                guard
                    .as_ref()
                    .filter(|session| session.session_id == expected)
                    .ok_or_else(|| "Liveセッションが切り替わりました".to_string())?
                    .started_at
            };
            let markdown = build_markdown(
                course,
                started_at,
                ended_at,
                &overall_summary,
                &snapshot.summaries,
                &snapshot.transcript_lines,
            );
            // Cache first: a crash before the markdown replacement must not
            // make a resumed session overwrite the only copy of newer speech.
            save_day_cache_full(
                course,
                started_at,
                &snapshot.transcript_lines,
                &snapshot.summaries,
            )?;
            let path = write_formal_markdown_file(course, started_at, &markdown)?;
            let mut guard = state
                .session
                .lock()
                .map_err(|_| "Live state lock failed".to_string())?;
            let session = guard
                .as_mut()
                .filter(|session| session.session_id == expected && session.finish_phase.is_some())
                .ok_or_else(|| "Liveセッションが切り替わりました".to_string())?;
            session.cache_progress =
                CacheProgress::committed(snapshot.transcript_lines.len(), snapshot.summaries.len());
            if clear {
                if session.transcript_lines.len() != snapshot.transcript_lines.len()
                    || session.summaries.len() != snapshot.summaries.len()
                {
                    continue;
                }
                snapshot = state.capture_completed_snapshot(session);
                *guard = None;
            }
            return Ok((path, markdown, snapshot));
        }
    })
    .await
    .map_err(|error| format!("Live保存処理失敗: {error}"))?
}

#[cfg(test)]
mod lifecycle_tests {
    use super::super::tests::transcript::recording;
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn startup_activity_reads_only_presence_and_never_captures_history_or_a_revision() {
        use tauri::ipc::IpcResponse;
        let state = LiveState::new();
        let inactive = active_session_reply(state.clone()).await.unwrap();
        assert!(!inactive.body().unwrap().deserialize::<bool>().unwrap());
        let mut session = recording();
        session.transcript_lines = Arc::new(
            (0..10_000)
                .map(|index| {
                    LiveTranscriptLine {
                        at: "10:00:00".into(),
                        text: format!("完全な記録 {index} 👩🏽‍💻"),
                    }
                    .into()
                })
                .collect(),
        );
        session.pending_lines = Arc::clone(&session.transcript_lines);
        let weak = Arc::downgrade(&session.transcript_lines);
        *state.session.lock().unwrap() = Some(session);
        let before = current_snapshot(&state);
        let active = active_session_reply(state.clone()).await.unwrap();
        assert!(
            matches!(active.body().unwrap(), tauri::ipc::InvokeResponseBody::Json(json) if json == "true")
        );
        assert_eq!(weak.strong_count(), 4); // state transcript/pending + before snapshot's two owners
        let after = current_snapshot(&state);
        assert_eq!(after.update_revision, before.update_revision + 1);
        assert_eq!(after.transcript_lines.len(), 10_000);
        assert_eq!(after.transcript_lines[9_999].text, "完全な記録 9999 👩🏽‍💻");
        drop(before);
        drop(after);
        *state.session.lock().unwrap() = None;
        assert_eq!(weak.strong_count(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn startup_activity_waits_for_a_busy_live_owner_and_does_not_block_the_async_executor() {
        use tauri::ipc::IpcResponse;
        let state = LiveState::new();
        *state.session.lock().unwrap() = Some(recording());
        let working = state.clone();
        let (locked, acquired) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _guard = working.session.lock().unwrap();
            locked.send(()).unwrap();
            released.recv().unwrap();
        });
        acquired.await.unwrap();
        let read = tokio::spawn(active_session_reply(state));
        assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
        assert!(!read.is_finished());
        release.send(()).unwrap();
        holder.join().unwrap();
        assert!(read
            .await
            .unwrap()
            .unwrap()
            .body()
            .unwrap()
            .deserialize::<bool>()
            .unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn startup_activity_reports_a_poisoned_owner_instead_of_false() {
        let state = LiveState::new();
        let working = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = working.session.lock().unwrap();
            panic!("poison test owner");
        })
        .join();
        assert_eq!(
            active_session_reply(state).await.err().unwrap(),
            "Live state lock failed"
        );
    }

    fn microphone(
        phase: crate::stt::SttStreamPhase,
        caller: &str,
        recording: &str,
    ) -> crate::stt::SttStreamState {
        crate::stt::SttStreamState {
            phase,
            session_id: Some(90),
            owner: Some(crate::stt::SttStreamOwner {
                caller: caller.into(),
                live_session_id: Some(recording.into()),
                input_session_id: None,
            }),
        }
    }

    #[test]
    fn inactive_tray_status_does_not_read_microphone_or_history() {
        let status = tray_status_snapshot(&LiveState::new(), || {
            panic!("inactive tray read microphone")
        })
        .unwrap();
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!({
                "active": false, "listening": false, "started_at": null,
            })
        );
    }

    #[test]
    fn tray_recording_status_matches_the_live_owner_and_actual_capture_phase() {
        use crate::stt::SttStreamPhase::*;
        let state = LiveState::new();
        *state.session.lock().unwrap() = Some(recording());
        for (phase, caller, id, expected) in [
            (Listening, "live", "recording-test", true),
            (Initializing, "live", "recording-test", false),
            (Stopping, "live", "recording-test", false),
            (Listening, "agent", "recording-test", false),
            (Listening, "live", "old-recording", false),
        ] {
            let status = tray_status_snapshot(&state, || {
                assert!(
                    state.session.try_lock().is_err(),
                    "LIVE owner was unlocked before microphone read"
                );
                Ok(microphone(phase, caller, id))
            })
            .unwrap();
            assert!(status.active);
            assert_eq!(status.listening, expected);
            assert!(status.started_at.is_some());
        }
    }

    #[test]
    fn tray_payload_size_stays_constant_as_the_recording_history_grows() {
        let state = LiveState::new();
        *state.session.lock().unwrap() = Some(recording());
        let read = || {
            Ok(microphone(
                crate::stt::SttStreamPhase::Listening,
                "live",
                "recording-test",
            ))
        };
        let before = serde_json::to_vec(&tray_status_snapshot(&state, read).unwrap()).unwrap();
        {
            let mut locked = state.session.lock().unwrap();
            let session = locked.as_mut().unwrap();
            session.transcript_lines = Arc::new(
                (0..10_000)
                    .map(|index| {
                        LiveTranscriptLine {
                            at: "10:00:00".into(),
                            text: format!("授業の確定字幕です。履歴を省略せず保存します。 {index}"),
                        }
                        .into()
                    })
                    .collect(),
            );
            session.pending_lines = Arc::clone(&session.transcript_lines);
        }
        let after = serde_json::to_vec(&tray_status_snapshot(&state, read).unwrap()).unwrap();
        assert_eq!(before, after);
        assert!(after.len() < 100);
        let full = serde_json::to_vec(&current_snapshot(&state)).unwrap();
        assert!(full.len() > 1_000_000);
        println!(
            "tray metadata: {} bytes; full LIVE snapshot with 10,000 lines: {} bytes",
            after.len(),
            full.len()
        );
        // Neither metadata read retained the growing history.
        assert_eq!(
            Arc::strong_count(
                &state
                    .session
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .transcript_lines
            ),
            2
        );
    }

    #[test]
    fn a_failed_microphone_read_is_an_error_instead_of_an_inactive_recording() {
        let state = LiveState::new();
        *state.session.lock().unwrap() = Some(recording());
        let error = tray_status_snapshot(&state, || Err("microphone state unavailable".into()))
            .unwrap_err();
        assert_eq!(error, "microphone state unavailable");
        assert!(state.is_session_current("recording-test"));
    }

    #[test]
    fn delayed_finish_and_cancel_cannot_mutate_a_replacement_recording() {
        let state = LiveState::new();
        let mut current = recording();
        current.session_id = "replacement".into();
        current.is_fresh_start = false; // No user storage is touched, even if this test regresses.
        current.append_line(LiveTranscriptLine {
            at: "10:00:00".into(),
            text: "new speech".into(),
        });
        let transcript = Arc::clone(&current.transcript_lines);
        *state.session.lock().unwrap() = Some(current);
        assert!(state.begin_finish("recording-test").is_err());
        assert!(cancel_session(&state, "recording-test").is_err());
        let snapshot = current_snapshot(&state);
        assert_eq!(snapshot.session_id.as_deref(), Some("replacement"));
        assert!(snapshot.finish_phase.is_none());
        assert_eq!(snapshot.finish_revision, 0);
        assert!(Arc::ptr_eq(&snapshot.transcript_lines, &transcript));
        assert_eq!(snapshot.transcript_lines[0].text, "new speech");
        let finish = state.begin_finish("replacement").unwrap();
        assert_eq!(
            current_snapshot(&state).finish_phase,
            Some(LiveFinishPhase::Stopping)
        );
        drop(finish);
        cancel_session(&state, "replacement").unwrap();
        assert!(state.active_session_id().is_none());
    }

    #[test]
    fn cancel_waiting_for_storage_revalidates_its_original_owner_after_the_wait() {
        let state = LiveState::new();
        let storage = state.persistence.gate.lock().unwrap();
        let pending_state = state.clone();
        let (started, ready) = std::sync::mpsc::channel();
        let pending = std::thread::spawn(move || {
            started.send(()).unwrap();
            cancel_session(&pending_state, "recording-test")
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let mut replacement = recording();
        replacement.session_id = "replacement".into();
        replacement.is_fresh_start = false;
        *state.session.lock().unwrap() = Some(replacement);
        drop(storage);
        assert!(pending.join().unwrap().is_err());
        assert_eq!(state.active_session_id().as_deref(), Some("replacement"));
    }

    #[test]
    fn microphone_admission_checks_the_owner_and_holds_it_through_the_short_reservation() {
        let state = LiveState::new();
        *state.session.lock().unwrap() = Some(recording());
        let mut reservations = 0;
        assert!(state
            .with_microphone_owner("old-recording", || {
                reservations += 1;
                Ok(())
            })
            .is_err());
        assert_eq!(reservations, 0);
        state
            .with_microphone_owner("recording-test", || {
                assert!(
                    state.session.try_lock().is_err(),
                    "validation must remain locked until reservation completes"
                );
                reservations += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(reservations, 1);
        let finish = state.begin_finish("recording-test").unwrap();
        assert!(state
            .with_microphone_owner("recording-test", || {
                reservations += 1;
                Ok(())
            })
            .is_err());
        assert_eq!(reservations, 1);
        drop(finish);
    }

    #[test]
    fn overall_summary_reads_only_the_requested_recording_and_keeps_its_snapshot_references() {
        let state = LiveState::new();
        let mut current = recording();
        current.append_line(LiveTranscriptLine {
            at: "10:00:00".into(),
            text: "owned speech".into(),
        });
        let transcript = Arc::clone(&current.transcript_lines);
        *state.session.lock().unwrap() = Some(current);
        assert!(overall_summary_input(&state, "old-recording").is_err());
        let (_, _, owned, _) = overall_summary_input(&state, "recording-test").unwrap();
        assert!(Arc::ptr_eq(&owned, &transcript));
        let finish = state.begin_finish("recording-test").unwrap();
        assert!(overall_summary_input(&state, "recording-test").is_err());
        drop(finish);
        let mut replacement = recording();
        replacement.session_id = "replacement".into();
        *state.session.lock().unwrap() = Some(replacement);
        assert_eq!(owned[0].text, "owned speech");
        assert!(overall_summary_input(&state, "recording-test").is_err());
    }

    #[test]
    fn finish_stages_survive_snapshot_recovery_and_failure_advances_the_revision() {
        let state = LiveState::new();
        *state.session.lock().unwrap() = Some(recording());
        assert!(state
            .set_finish_phase("recording-test", LiveFinishPhase::SavingFinal)
            .is_err());
        let ownership = state.begin_finish("recording-test").unwrap();
        assert_eq!(
            current_snapshot(&state).finish_phase,
            Some(LiveFinishPhase::Stopping)
        );
        let mut revision = current_snapshot(&state).finish_revision;
        for (phase, wire_name) in [
            (LiveFinishPhase::Stopping, "stopping"),
            (LiveFinishPhase::SavingRecord, "saving_record"),
            (LiveFinishPhase::Summarizing, "summarizing"),
            (LiveFinishPhase::SavingFinal, "saving_final"),
        ] {
            let progress = state.set_finish_phase("recording-test", phase).unwrap();
            assert!(progress.finish_revision > revision);
            revision = progress.finish_revision;
            let serialized = serde_json::to_value(current_snapshot(&state)).unwrap();
            assert_eq!(serialized["finish_phase"].as_str(), Some(wire_name));
            let recovered: LiveSessionSnapshot = serde_json::from_value(serialized).unwrap();
            assert_eq!(recovered.finish_phase, Some(phase));
            assert_eq!(recovered.finish_revision, revision);
            assert!(recovered.next_summary_at_ms.is_none());
            assert!(state
                .with_microphone_owner("recording-test", || Ok(()))
                .is_err());
            assert!(serde_json::to_value(progress)
                .unwrap()
                .get("transcript_lines")
                .is_none());
        }
        assert!(state
            .set_finish_phase("old-recording", LiveFinishPhase::SavingFinal)
            .is_err());
        drop(ownership);
        let failed = current_snapshot(&state);
        assert!(failed.finish_phase.is_none());
        assert!(failed.finish_revision > revision);
        assert!(state
            .with_microphone_owner("recording-test", || Ok(()))
            .is_ok());
        let retry = state.begin_finish("recording-test").unwrap();
        assert!(current_snapshot(&state).finish_revision > failed.finish_revision);
        drop(retry);
    }

    #[test]
    fn completed_snapshot_keeps_saved_content_without_appearing_to_be_an_active_save() {
        let mut session = recording();
        session.finish_phase = Some(LiveFinishPhase::SavingFinal);
        session.finish_revision = 4;
        session.append_line(LiveTranscriptLine {
            at: "10:00:00".into(),
            text: "saved speech".into(),
        });
        let saved = session.completed_snapshot();
        assert_eq!(saved.session_id.as_deref(), Some("recording-test"));
        assert!(!saved.active);
        assert!(saved.finish_phase.is_none());
        assert_eq!(saved.finish_revision, 5);
        assert_eq!(saved.transcript_lines[0].text, "saved speech");
        assert!(Arc::ptr_eq(
            &saved.transcript_lines,
            &session.transcript_lines
        ));
        let mut legacy = serde_json::to_value(saved).unwrap();
        legacy.as_object_mut().unwrap().remove("finish_phase");
        legacy.as_object_mut().unwrap().remove("finish_revision");
        let restored: LiveSessionSnapshot = serde_json::from_value(legacy).unwrap();
        assert!(restored.finish_phase.is_none());
        assert_eq!(restored.finish_revision, 0);
    }

    #[test]
    fn finish_reserves_the_recording_but_accepts_the_decoder_tail_and_can_retry() {
        let state = LiveState::new();
        *state.session.lock().unwrap() = Some(recording());
        let ownership = state.begin_finish("recording-test").unwrap();
        assert!(state.begin_finish("recording-test").is_err());
        assert!(state
            .with_microphone_owner("recording-test", || Ok(()))
            .is_err());
        assert!(state
            .with_microphone_owner("recording-test", || Ok(()))
            .is_err());
        assert!(cancel_session(&state, "recording-test").is_err());
        let course = state
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .course
            .clone();
        assert!(start_session(&state, course).is_err());
        let tail = LiveTranscriptLine {
            at: "10:00:00".into(),
            text: "decoder tail".into(),
        };
        assert!(state.append_line_for_session(None, tail.clone()).is_err());
        assert!(state
            .append_line_for_session(Some("recording-test"), tail)
            .unwrap()
            .is_some());
        drop(ownership);
        assert!(state
            .with_microphone_owner("recording-test", || Ok(()))
            .is_ok());
        assert!(state.begin_finish("recording-test").is_ok());
        assert_eq!(
            state
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .transcript_lines[0]
                .text,
            "decoder tail"
        );
    }

    #[test]
    fn old_finish_cleanup_cannot_release_a_new_recordings_reservation() {
        let state = LiveState::new();
        *state.session.lock().unwrap() = Some(recording());
        let old = state.begin_finish("recording-test").unwrap();
        let mut replacement = recording();
        replacement.session_id = "next-recording".into();
        *state.session.lock().unwrap() = Some(replacement);
        let current = state.begin_finish("next-recording").unwrap();
        drop(old);
        assert!(state
            .with_microphone_owner("next-recording", || Ok(()))
            .is_err());
        drop(current);
        assert!(state
            .with_microphone_owner("next-recording", || Ok(()))
            .is_ok());
    }
}
