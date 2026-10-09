use super::*;
use crate::live::tests::transcript::recording;
use chrono::TimeZone;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "legacy.rs"]
mod legacy;

fn fixture(count: usize, free: bool) -> Captured {
    let mut session = recording();
    session.course.course_name = "講義 / 自由記録 🌕".into();
    session.course.course_code = "CODE".into();
    session.course.day = 4;
    session.course.period = 2;
    session.course.is_free_note = free;
    for i in 0..count {
        session.append_line(LiveTranscriptLine {
            text: format!("[line {i}] 日本語\n\"quoted\" 👩🏽‍💻　"),
            at: format!("10:{:02}:00", i % 60),
        });
    }
    for i in 0..7 {
        session.append_summary(LiveSummaryChunk {
            title: format!("要約 {i}"),
            range_label: "10:00-10:10".into(),
            body: format!("完全な本文 {i}\n{}", "引用 🌕".repeat(32)),
            line_count: i + 1,
            terms: vec![LiveTermExplanation {
                term: format!("概念{i}"),
                explanation: "全用語説明".into(),
                source_excerpt: "発話引用".into(),
                external_source: "外部資料".into(),
            }],
            whiteboard: (i == 6).then(|| serde_json::from_value(serde_json::json!({
                "title": "累積ボード 🌕", "layout": "grid",
                "nodes": [
                    {"id":"main", "label":"一次資料", "detail":"引用を保持する", "node_type":"structure", "kind":"core", "role":"main", "source_type":"lecture"},
                    {"id":"branch", "label":"参考資料", "node_type":"structure", "role":"branch", "parent_id":"main", "source_type":"lecture"},
                    {"id":"term", "label":"用語の説明", "node_type":"term", "role":"branch", "parent_id":"main", "source_type":"lecture"}
                ],
                "edges":[{"from":"main","to":"branch","label":"参考"}]
            })).unwrap()),
        });
    }
    Captured::new(
        &session.course,
        &session.transcript_lines,
        &session.summaries,
    )
}
fn cfg(language: &str) -> crate::ai::AiConfig {
    crate::ai::AiConfig {
        ai_enabled: true,
        provider: "openai".into(),
        api_key: "fixture-only".into(),
        reply_language: language.into(),
        ..Default::default()
    }
}
fn value(messages: Vec<crate::ai::ChatMessage>) -> serde_json::Value {
    serde_json::to_value(messages).unwrap()
}
fn summary() -> LiveChunkAiResult {
    parse_chunk_ai_result("{\"summary_markdown\":\"# 新しい要約\\n引用 🌕\",\"terms\":[{\"term\":\"一次資料\",\"explanation\":\"大元の資料\"}]}")
}

#[test]
fn all_request_messages_match_previous_text_at_tail_boundaries_and_reply_languages() {
    for language in ["ja", "zh", "en", "ko"] {
        for free in [false, true] {
            for count in [0, 1, 24, 25, 80, 81, 500, 501, 1000] {
                let input = fixture(count, free);
                let config = cfg(language);
                let parsed = summary();
                let (old_first, old_board) = legacy::chunk(&config, &input, &parsed, "10:00-10:10");
                assert_eq!(value(chunk_messages(&config, &input)), value(old_first));
                assert_eq!(
                    value(whiteboard_messages(
                        language,
                        &input,
                        &parsed,
                        "10:00-10:10"
                    )),
                    value(old_board)
                );
                assert_eq!(
                    value(overall_messages(&config, &input)),
                    value(legacy::overall(&config, &input))
                );
                assert_eq!(
                    value(todo_messages(&config, &input, "計画\n提出 🌕")),
                    value(legacy::todo(&config, &input, "計画\n提出 🌕"))
                );
            }
        }
    }
    // Add full teachers/rooms as well as the fixture's empty/default labels.
    let mut input = fixture(501, false);
    input.course.teacher = "教員名 👩🏽‍💻".into();
    input.course.room = "教室 B201".into();
    let config = cfg("en");
    let parsed = summary();
    let (first, board) = legacy::chunk(&config, &input, &parsed, "12:00-12:30");
    assert_eq!(value(chunk_messages(&config, &input)), value(first));
    assert_eq!(
        value(whiteboard_messages("en", &input, &parsed, "12:00-12:30")),
        value(board)
    );
    assert_eq!(
        value(overall_messages(&config, &input)),
        value(legacy::overall(&config, &input))
    );
    assert_eq!(
        value(todo_messages(&config, &input, "plan")),
        value(legacy::todo(&config, &input, "plan"))
    );
}

#[test]
fn overall_gates_validate_one_settings_snapshot_and_keep_short_local_and_unavailable_fallbacks() {
    let input = fixture(100, false);
    let started = Local
        .with_ymd_and_hms(2026, 10, 8, 10, 0, 0)
        .single()
        .unwrap();
    for language in ["ja", "zh", "en", "ko"] {
        let config = cfg(language);
        let short = overall_with(
            &input,
            started,
            started + ChronoDuration::seconds(119),
            config.clone(),
            |_| panic!("short recording validated a model request"),
        );
        let Overall::Ready(text) = short else {
            panic!("short recording submitted a model request")
        };
        assert_eq!(
            text,
            short_session_overall_summary(&input.course, 100, language)
        );
        let mut local = config.clone();
        local.provider = "local".into();
        let local = overall_with(
            &input,
            started,
            started + ChronoDuration::minutes(10),
            local,
            |_| panic!("local finish validated a model request"),
        );
        let Overall::Ready(text) = local else {
            panic!("local finish submitted a model request")
        };
        assert_eq!(
            text,
            fallback_overall_summary(&input.course, 100, 7, language)
        );
        for missing_key in [false, true] {
            let mut unavailable = config.clone();
            if missing_key {
                unavailable.api_key.clear();
            } else {
                unavailable.ai_enabled = false;
            }
            let unavailable = overall_with(
                &input,
                started,
                started + ChronoDuration::minutes(10),
                unavailable,
                validate_live_ai_config,
            );
            let Overall::Ready(text) = unavailable else {
                panic!("invalid settings submitted a model request")
            };
            assert_eq!(
                text,
                fallback_overall_summary(&input.course, 100, 7, language)
            );
        }
        let failed = overall_with(
            &input,
            started,
            started + ChronoDuration::minutes(10),
            config.clone(),
            |_| Err("original configuration failure".into()),
        );
        let Overall::Ready(text) = failed else {
            panic!("failed configuration submitted a model request")
        };
        assert_eq!(
            text,
            fallback_overall_summary(&input.course, 100, 7, language)
        );
        let count = AtomicUsize::new(0);
        let model = overall_with(
            &input,
            started,
            started + ChronoDuration::minutes(10),
            config,
            |resolved| {
                count.fetch_add(1, Ordering::SeqCst);
                assert_eq!(resolved.reply_language, language);
                validate_live_ai_config(resolved)
            },
        );
        let Overall::Model { request, fallback } = model else {
            panic!("cloud finish skipped its model request")
        };
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(request.cfg.reply_language, language);
        assert_eq!(
            value(request.messages),
            value(overall_messages(&cfg(language), &input))
        );
        assert_eq!(
            fallback,
            fallback_overall_summary(&input.course, 100, 7, language)
        );
    }
}

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn targeted_course_context_and_complete_todo_messages_match_the_previous_lookup() {
    let temporary = Temporary(std::env::temp_dir().join(format!(
        "selah-live-course-context-{}",
        uuid::Uuid::new_v4()
    )));
    let db = crate::db::Database::open(&temporary.0).unwrap();
    for count in [1, 18, 19, 30] {
        for alias in [false, true] {
            let code = format!("CODE-{count}-{alias}");
            let stored = if alias {
                format!("　\t{code}\u{2003}")
            } else {
                code.clone()
            };
            let plans = (0..count)
                .rev()
                .map(|n| crate::db::SessionPlanRow {
                    session_num: n,
                    th_header: if n == 0 {
                        "　\t".into()
                    } else {
                        format!("第{n}回 👩🏽‍💻").repeat(40)
                    },
                    topic: if n == 0 {
                        "".into()
                    } else {
                        format!("研究テーマ {n}\n\"引用\" \\").repeat(50)
                    },
                    delivery_mode: if n % 2 == 0 { "offline" } else { "online" }.into(),
                    study_outside: if n == 0 {
                        "\n".into()
                    } else {
                        "提出と授業外学修 🌕 ".repeat(50)
                    },
                })
                .collect::<Vec<_>>();
            db.upsert_session_plans(&stored, &plans).unwrap();
            db.upsert_kgc_course_detail(&crate::db::KgcCourseDetailRow {
                kgc_code: code.clone(),
                fields: vec![
                    ("評価方法".into(), "　\t".into()),
                    ("参考資料".into(), "対象外の補足".into()),
                    ("授業外学修".into(), "Unicode 👩🏽‍💻\n\"引用\"　".repeat(100)),
                    ("課題提出".into(), "完全な提出方法".into()),
                    ("評価基準".into(), "正しい引用".into()),
                    ("試験の詳細".into(), "準備資料".into()),
                    ("評価補足".into(), "既存の四項目上限で対象外".into()),
                ],
                delivery_mode: "offline".into(),
                textbooks: Vec::new(),
            })
            .unwrap();
        }
    }
    let ended = Local
        .with_ymd_and_hms(2026, 12, 31, 23, 59, 0)
        .single()
        .unwrap();
    for count in [0, 1, 18, 19, 30] {
        for alias in [false, true] {
            for (day, period) in [(0, -1), (1, 1), (4, 2), (7, 8), (8, 0)] {
                let mut input = fixture(81, false);
                input.course.course_code = format!("\u{2003} CODE-{count}-{alias}\t　");
                input.course.day = day;
                input.course.period = period;
                let old = legacy::course_plan_context(&db, &input.course, ended);
                let actual = live_todo_course_plan_context(&db, &input.course, ended);
                assert_eq!(
                    actual, old,
                    "count={count}, alias={alias}, day={day}, period={period}"
                );
                for language in ["ja", "zh", "en", "ko"] {
                    assert_eq!(
                        value(todo_messages(&cfg(language), &input, &actual)),
                        value(legacy::todo(&cfg(language), &input, &old))
                    );
                }
            }
        }
    }
    let mut input = fixture(1, false);
    input.course.course_code = "　\t".into();
    assert_eq!(
        live_todo_course_plan_context(&db, &input.course, ended),
        legacy::course_plan_context(&db, &input.course, ended)
    );
}

#[test]
fn corrupted_target_plan_is_a_read_failure_and_syllabus_supplement_is_preserved() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-live-plan-error-{}", uuid::Uuid::new_v4())),
    );
    let db = crate::db::Database::open(&temporary.0).unwrap();
    db.upsert_session_plans(
        "CODE",
        &[crate::db::SessionPlanRow {
            session_num: 1,
            th_header: "概要".into(),
            topic: "元の内容".into(),
            delivery_mode: "offline".into(),
            study_outside: "提出資料".into(),
        }],
    )
    .unwrap();
    db.upsert_kgc_course_detail(&crate::db::KgcCourseDetailRow {
        kgc_code: "CODE".into(),
        fields: vec![("評価方法".into(), "独立した補足".into())],
        delivery_mode: "offline".into(),
        textbooks: Vec::new(),
    })
    .unwrap();
    let external = rusqlite::Connection::open(temporary.0.join("courses.db")).unwrap();
    external
        .execute(
            "UPDATE session_plans SET topic=x'ff' WHERE kgc_code='CODE'",
            [],
        )
        .unwrap();
    let input = fixture(1, false);
    let ended = Local
        .with_ymd_and_hms(2026, 10, 8, 10, 0, 0)
        .single()
        .unwrap();
    let actual = live_todo_course_plan_context(&db, &input.course, ended);
    assert!(actual.contains("授業計画: 読み込み失敗"));
    assert!(actual.contains("シラバス補足:\n評価方法: 独立した補足"));
    assert!(!actual.contains("キャッシュなし"));
    assert!(!actual.contains("第1回:"));
    external.execute_batch("DROP TABLE session_plans;").unwrap();
    assert_eq!(
        live_todo_course_plan_context(&db, &input.course, ended),
        legacy::course_plan_context(&db, &input.course, ended)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn delayed_preparation_leaves_executor_and_live_state_available_and_uses_captured_inputs() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-live-requests-{}", uuid::Uuid::new_v4())),
    );
    let database = crate::db::Database::open(&temporary.0).unwrap();
    database
        .upsert_session_plans(
            "CODE",
            &[crate::db::SessionPlanRow {
                session_num: 1,
                th_header: "概要".into(),
                topic: "レポート提出 🌕".into(),
                delivery_mode: "offline".into(),
                study_outside: "完全な学修計画".into(),
            }],
        )
        .unwrap();
    database
        .upsert_kgc_course_detail(&crate::db::KgcCourseDetailRow {
            kgc_code: "CODE".into(),
            fields: vec![("評価方法".into(), "引用資料の提出".into())],
            delivery_mode: "offline".into(),
            textbooks: vec![],
        })
        .unwrap();
    let state = LiveState::new();
    let mut session = recording();
    session.course.course_code = "CODE".into();
    session.append_line(LiveTranscriptLine {
        text: "accepted speech 🌕".into(),
        at: "10:00:00".into(),
    });
    let captured = Captured::new(
        &session.course,
        &session.transcript_lines,
        &session.summaries,
    );
    let history_weak = Arc::downgrade(&session.transcript_lines);
    *state.session.lock().unwrap() = Some(session);
    let caller = std::thread::current().id();
    let entered_state = state.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let end = Local
        .with_ymd_and_hms(2026, 10, 8, 10, 0, 0)
        .single()
        .unwrap();
    let task = tokio::spawn(prepare(captured, move |input| {
        assert_ne!(std::thread::current().id(), caller);
        assert!(entered_state.session.try_lock().is_ok());
        assert!(entered_state.persistence.gate.try_lock().is_ok());
        entered.send(()).unwrap();
        released.recv().unwrap();
        let context = live_todo_course_plan_context(&database, &input.course, end);
        Ok(todo_messages(&cfg("zh"), input, &context))
    }));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    assert!(!task.is_finished());
    state
        .append_line_for_session(
            Some("recording-test"),
            LiveTranscriptLine {
                text: "later speech must not enter this prompt".into(),
                at: "10:00:01".into(),
            },
        )
        .unwrap();
    *state.session.lock().unwrap() = None;
    release.send(()).unwrap();
    let messages = task.await.unwrap().unwrap();
    let text = &messages[1].content;
    assert!(text.contains("accepted speech 🌕"));
    assert!(!text.contains("later speech"));
    assert!(text.contains("レポート提出 🌕"));
    assert!(text.contains("引用資料の提出"));
    assert!(history_weak.upgrade().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn whiteboard_preparation_preserves_owned_summary_and_join_errors_are_labelled() {
    let parsed = summary();
    let pointer = parsed.body.as_ptr();
    let expected = result_value(&parsed);
    let (parsed, messages) = whiteboard(
        fixture(501, false),
        "zh".into(),
        parsed,
        "10:00-10:10".into(),
    )
    .await
    .unwrap();
    assert_eq!(parsed.body.as_ptr(), pointer);
    assert_eq!(result_value(&parsed), expected);
    assert!(messages[1].content.contains("古い文字起こし 1 行を省略"));
    assert!(messages[1].content.contains("[line 500]"));
    assert!(!messages[1].content.contains("[line 0]"));
    let failed = prepare(fixture(0, false), |_| -> Result<(), String> {
        panic!("fixture worker panic")
    })
    .await;
    assert!(failed.err().unwrap().starts_with(FAILURE));
}

fn result_value(parsed: &LiveChunkAiResult) -> serde_json::Value {
    serde_json::json!({"body":parsed.body,"terms":parsed.terms,"whiteboard":parsed.whiteboard})
}
