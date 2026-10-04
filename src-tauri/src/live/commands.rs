//! Tauri commands for the LIVE session.
//!
//! Handlers translate UI actions into session changes. Summary generation and
//! the flush driver stay beside the session state.

use super::*;

#[tauri::command]
pub fn live_get_session(state: tauri::State<'_, LiveState>) -> LiveSessionSnapshot {
    current_snapshot(&state)
}

/// Peek at the day cache for a course without starting a session.
/// Returns an inactive snapshot with the cached transcript/summaries, or empty if no cache.
#[tauri::command]
pub fn live_peek_day_cache(course: LiveCourseInfo) -> LiveSessionSnapshot {
    match load_day_cache(&course) {
        Some(cache) => LiveSessionSnapshot {
            active: false,
            course: Some(course),
            started_at: Some(cache.started_at),
            transcript_lines: Arc::new(cache.transcript_lines),
            pending_lines: Arc::new(Vec::new()),
            summaries: Arc::new(cache.summaries),
            next_summary_at_ms: None,
            summarizing: false,
        },
        None => empty_snapshot(),
    }
}

#[tauri::command]
pub fn live_start_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    mut course: LiveCourseInfo,
) -> Result<LiveSessionSnapshot, String> {
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

    let persisted_line_count = prev_transcript.len();
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
        persisted_line_count,
    };
    let snapshot = session.snapshot();
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    *guard = Some(session);
    drop(guard);
    emit_live_update(&app, &state);
    start_live_flush_driver(app.clone(), session_id);
    Ok(snapshot)
}

#[tauri::command]
pub fn live_append_transcript(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    text: String,
) -> Result<LiveSessionSnapshot, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(current_snapshot(&state));
    }
    let line = LiveTranscriptLine {
        text: text.to_string(),
        at: Local::now().format("%H:%M:%S").to_string(),
    };
    let snapshot = {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_mut()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
        // make_mut is in-place when no other Arc holders exist; if a
        // previously-emitted snapshot is still being serialized it copies once
        // — bounded and rare. Either way, no per-append deep clone of the Vec.
        Arc::make_mut(&mut session.transcript_lines).push(line.clone());
        Arc::make_mut(&mut session.pending_lines).push(line.clone());
        session.snapshot()
    };
    auto_save_day_cache(&state, false);
    state.inner().notify_flush_driver();
    // Slim delta event for the subtitle overlay and any cheap subscriber.
    // Emitting the full snapshot per final line grew O(N) in payload size —
    // a 2-hour lecture was serialising hundreds of KB on every append.
    let _ = app.emit("live-line-appended", &line);
    Ok(snapshot)
}

#[tauri::command]
pub async fn live_flush_summary(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    force: bool,
) -> Result<LiveSessionSnapshot, String> {
    live_flush_summary_with_side_effects(&app, state.inner(), force).await
}

#[tauri::command]
pub async fn live_generate_overall_summary(
    state: tauri::State<'_, LiveState>,
) -> Result<String, String> {
    // The overall summary is a quick "so far" snapshot: it reads the existing
    // summary chunks plus the tail of the live transcript (which already holds
    // every appended line, pending ones included). It must NOT force a chunk
    // flush first — that ran the full per-chunk pipeline (summary + terms +
    // whiteboard generation, several AI calls) and could block up to
    // LIVE_FLUSH_FORCE_WAIT_ATTEMPTS while a scheduled flush was in flight,
    // which is what made this button take abnormally long.
    let (course, started_at, transcript_lines, summaries) = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
        if session.transcript_lines.is_empty() {
            return Err("全体要約を生成できる文字起こしがまだありません".to_string());
        }
        (
            session.course.clone(),
            session.started_at,
            session.transcript_lines.clone(),
            session.summaries.clone(),
        )
    };

    Ok(generate_overall_summary(
        &course,
        started_at,
        Local::now(),
        &summaries,
        &transcript_lines,
    )
    .await)
}

#[tauri::command]
pub fn live_cancel_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
) -> Result<(), String> {
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
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
    *guard = None;
    drop(guard);

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
                remove_day_cache(&course);
            }
        }
    }

    state.inner().notify_flush_driver();
    emit_live_update(&app, &state);
    Ok(())
}

/// Clear the day cache for a specific course, removing all accumulated transcript/summary data.
#[tauri::command]
pub fn live_clear_day_cache(course: LiveCourseInfo) -> Result<(), String> {
    if course.is_free_note {
        return Ok(());
    }
    if course.course_name.trim().is_empty() {
        return Err("講義名が空です".into());
    }
    remove_day_cache(&course);
    Ok(())
}

#[tauri::command]
pub async fn live_finish_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
) -> Result<LiveSaveResult, String> {
    let (course, started_at, transcript_lines, summaries, pending_line_count) = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let Some(session) = guard.as_ref() else {
            return Err("Liveセッションが開始されていません".to_string());
        };
        if session.transcript_lines.is_empty() {
            let course = session.course.clone();
            drop(guard);
            if !course.is_free_note {
                remove_day_cache(&course);
            }
            let snapshot = {
                let mut guard = state
                    .0
                    .lock()
                    .map_err(|_| "Live state lock failed".to_string())?;
                let session = guard
                    .as_ref()
                    .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
                let snapshot = session.snapshot();
                *guard = None;
                snapshot
            };
            state.inner().notify_flush_driver();
            let result = LiveSaveResult {
                saved: false,
                path: String::new(),
                markdown: String::new(),
                snapshot,
                suggested_todos: Vec::new(),
                todos_pending: false,
            };
            emit_live_update(&app, &state);
            return Ok(result);
        }
        (
            session.course.clone(),
            session.started_at,
            session.transcript_lines.clone(),
            session.summaries.clone(),
            session.pending_lines.len(),
        )
    };

    let pre_ai_saved_at = Local::now();
    let pre_ai_markdown = build_markdown(
        &course,
        started_at,
        pre_ai_saved_at,
        PRE_AI_OVERALL_SUMMARY,
        &summaries,
        &transcript_lines,
    );
    write_formal_markdown_file(&course, started_at, &pre_ai_markdown)?;
    save_day_cache_full(&course, started_at, &transcript_lines, &summaries);
    emit_live_finish_progress(&app, "record_saved");

    let ai_config = crate::ai::load_ai_config();
    let will_run_finish_ai = should_run_finish_ai(&ai_config.provider, started_at, pre_ai_saved_at);
    let needs_final_chunk_ai =
        should_require_finish_chunk_ai(started_at, pre_ai_saved_at, pending_line_count);
    if needs_final_chunk_ai || will_run_finish_ai {
        emit_live_finish_progress(&app, "ai");
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
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
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
    let markdown = build_markdown(
        &course,
        started_at,
        ended_at,
        &overall_summary,
        &summaries,
        &transcript_lines,
    );
    // TODO/DDL judgment is the slowest AI step. It must NOT block saving — we
    // run it in the background (after the file is written and the session is
    // cleared) and push the result through the `live-todo-suggestions` event so
    // the UI can jump straight to the TODO page and add them there.
    let run_todos_in_background = !course.is_free_note
        && !transcript_lines.is_empty()
        && !should_skip_ai_summarization(started_at, ended_at)
        && should_run_finish_ai;

    emit_live_finish_progress(&app, "final_save");
    let path = write_formal_markdown_file(&course, started_at, &markdown)?;

    // Save day cache so next session for same course today can resume
    save_day_cache_full(&course, started_at, &transcript_lines, &summaries);

    let path_str = path.to_string_lossy().to_string();

    let snapshot = {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
        let snapshot = session.snapshot();
        *guard = None;
        snapshot
    };
    state.inner().notify_flush_driver();

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

    let result = LiveSaveResult {
        saved: true,
        path: path_str.clone(),
        markdown,
        snapshot,
        suggested_todos: Vec::new(),
        todos_pending: run_todos_in_background,
    };
    let _ = app.emit("live-session-saved", &result);
    emit_live_update(&app, &state);
    Ok(result)
}
