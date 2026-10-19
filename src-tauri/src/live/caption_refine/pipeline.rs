//! Corrections are checked before the translation request is even constructed.
use super::*;
use std::future::Future;
use tokio::time::{timeout_at, Instant as Deadline};

// Reserve half of the shared model budget for translating. A failed correction
// keeps the original source; a failed translation keeps the accepted correction.
const CORRECTION_BUDGET: Duration = Duration::from_secs(4);

pub(super) async fn request(job: CaptionRequest) -> Result<ParsedCaption, String> {
    request_with(
        job,
        Deadline::now() + MODEL_BUDGET,
        |config, messages| async move { crate::ai::caption_completion(&config, messages).await },
    )
    .await
}

async fn request_with<F, R>(
    job: CaptionRequest,
    deadline: Deadline,
    mut complete: F,
) -> Result<ParsedCaption, String>
where
    F: FnMut(crate::ai::AiConfig, Vec<crate::ai::ChatMessage>) -> R,
    R: Future<Output = Result<String, String>>,
{
    let correction_deadline = if job.translate {
        deadline.min(Deadline::now() + CORRECTION_BUDGET)
    } else {
        deadline
    };
    let messages = build_messages(&job, &job.caption, false)?;
    let mut config = job.config.clone();
    config.max_tokens = caption_output_budget(&job.caption, false);
    let correction = timeout_at(correction_deadline, complete(config, messages)).await;
    let mut parsed = match correction {
        Ok(Ok(raw)) => parse_model_correction(&raw, &job.caption),
        Ok(Err(err)) => {
            log::warn!("[Live] caption correction failed: {err}");
            None
        }
        Err(_) => {
            log::debug!("[Live] caption correction reached its deadline");
            None
        }
    }
    .unwrap_or_else(|| original_caption(&job.caption));
    if parsed.reverted_parts > 0 {
        log::debug!(
            "[Live] caption validation kept {} original sentences",
            parsed.reverted_parts
        );
    }
    if !job.translate || Deadline::now() >= deadline {
        return Ok(parsed);
    }

    // Construct the second call only from the accepted result. Neither the raw
    // alternatives nor a rejected edit is available to the translator.
    let source = translation_source(&parsed);
    let messages = match build_messages(&job, &source, true) {
        Ok(messages) => messages,
        Err(err) => {
            log::debug!("[Live] caption translation skipped: {err}");
            return Ok(parsed);
        }
    };
    let mut config = job.config.clone();
    config.max_tokens = caption_output_budget(&source, true);
    match timeout_at(deadline, complete(config, messages)).await {
        Ok(Ok(raw)) => {
            if apply_model_translation(&mut parsed, &raw, Some(job.target_language)).is_none() {
                log::debug!("[Live] caption translation failed validation");
            }
        }
        Ok(Err(err)) => log::warn!("[Live] caption translation failed: {err}"),
        Err(_) => log::debug!("[Live] caption translation reached the shared deadline"),
    }
    Ok(parsed)
}

fn build_messages(
    job: &CaptionRequest,
    caption: &str,
    translate: bool,
) -> Result<Vec<crate::ai::ChatMessage>, String> {
    let system = system_prompt(translate, language_label(job.target_language));
    let input = user_prompt(
        &job.course_name,
        &job.recent,
        &job.glossary,
        &job.lesson_terms,
        caption,
        language_label(&job.source_language),
        language_label(job.target_language),
        translate,
    );
    let input = fit_caption_input(&system, input, job.config.provider == "local")?;
    Ok(vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: system,
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: input,
            images: Vec::new(),
        },
    ])
}

fn original_caption(source: &str) -> ParsedCaption {
    let parts = protocol::parts(source);
    ParsedCaption {
        corrected: parts.join(" "),
        translation: None,
        terms: Vec::new(),
        reverted_parts: parts.len(),
        sentences: parts
            .into_iter()
            .map(|source| RefinedSentence {
                corrected: source.clone(),
                source,
                translation: None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn job(source: &str) -> CaptionRequest {
        let mut glossary = Glossary::default();
        glossary.learn("確立変数".into(), "確率変数".into());
        CaptionRequest {
            config: crate::ai::AiConfig {
                ai_enabled: true,
                temperature: 0.0,
                reply_language: "zh".into(),
                ..Default::default()
            },
            course_name: "確率論".into(),
            glossary,
            recent: vec!["前の説明は平均と分散です。".into()],
            lesson_terms: "確率変数: 確率で値が決まる変数".into(),
            caption: source.into(),
            source_language: "ja".into(),
            target_language: "zh",
            translate: true,
        }
    }

    fn input(messages: &[crate::ai::ChatMessage]) -> Value {
        serde_json::from_str(&messages.last().unwrap().content).unwrap()
    }

    #[tokio::test]
    async fn translator_receives_accepted_terms_and_reverted_facts_in_sentence_order() {
        let source = "確立変数を定義します。\n60点以上なら合格です。";
        let mut calls = Vec::new();
        let parsed = request_with(
            job(source),
            Deadline::now() + MODEL_BUDGET,
            |_, messages| {
                let value = input(&messages);
                calls.push(value.clone());
                let response = if calls.len() == 1 {
                    assert!(value.get("訳語").is_none());
                    assert_eq!(value["過去の訂正候補"], "確立変数→確率変数");
                    json!({"edits":[
                    {"id":1,"from":"確立変数","to":"確率変数"},
                    {"id":2,"from":"60点以上","to":"60点以下"}
                ], "translations":[{"id":1,"text":"THIS OLD TRANSLATION MUST NOT LEAK"}]})
                } else {
                    assert_eq!(
                        value["字幕"],
                        json!([
                            {"id":1,"text":"確率変数を定義します。"},
                            {"id":2,"text":"60点以上なら合格です。"}
                        ])
                    );
                    assert_eq!(value.as_object().unwrap().len(), 3);
                    assert!(!messages[1].content.contains("確立変数"));
                    assert!(!messages[1].content.contains("以下"));
                    json!({"translations":[
                        {"id":1,"text":"定义随机变量。"},
                        {"id":2,"text":"60分及以上即可通过。"}
                    ]})
                };
                std::future::ready(Ok(response.to_string()))
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(parsed.reverted_parts, 1);
        assert_eq!(parsed.sentences[0].source, "確立変数を定義します。");
        assert_eq!(parsed.sentences[0].corrected, "確率変数を定義します。");
        assert_eq!(
            parsed.sentences[0].translation.as_deref(),
            Some("定义随机变量。")
        );
        assert_eq!(parsed.sentences[1].corrected, "60点以上なら合格です。");
        assert_eq!(
            parsed.sentences[1].translation.as_deref(),
            Some("60分及以上即可通过。")
        );
        assert_eq!(
            parsed.translation.as_deref(),
            Some("定义随机变量。 60分及以上即可通过。")
        );
        assert_eq!(parsed.terms, vec![("確立変数".into(), "確率変数".into())]);
    }

    #[tokio::test]
    async fn malformed_correction_falls_back_before_constructing_the_translation() {
        for correction in [
            Err("unavailable".into()),
            Ok("invalid json".into()),
            Ok(json!({"edits":[{"id":2,"from":"確立変数","to":"確率変数"}]}).to_string()),
        ] {
            let mut calls = 0;
            let parsed = request_with(
                job("確立変数を定義します。"),
                Deadline::now() + MODEL_BUDGET,
                |_, messages| {
                    calls += 1;
                    std::future::ready(if calls == 1 {
                        correction.clone()
                    } else {
                        assert_eq!(
                            input(&messages)["字幕"][0]["text"],
                            "確立変数を定義します。"
                        );
                        Ok(json!({"translations":[{"id":1,"text":"定义确立变量。"}]}).to_string())
                    })
                },
            )
            .await
            .unwrap();
            assert_eq!(calls, 2);
            assert_eq!(parsed.corrected, "確立変数を定義します。");
            assert_eq!(parsed.translation.as_deref(), Some("定义确立变量。"));
            assert!(parsed.terms.is_empty());
        }
    }

    #[tokio::test]
    async fn translation_failure_keeps_the_accepted_correction_without_old_translation() {
        for response in [
            Err("unavailable".into()),
            Ok("invalid json".into()),
            Ok(json!({"translations":[{"id":2,"text":"定义随机变量。"}]}).to_string()),
            Ok(json!({"source_translations":[{"id":1,"text":"定义确立变量。"}]}).to_string()),
        ] {
            let mut calls = 0;
            let parsed = request_with(
                job("確立変数を定義します。"),
                Deadline::now() + MODEL_BUDGET,
                |_, _| {
                    calls += 1;
                    std::future::ready(if calls == 1 {
                        Ok(
                            json!({"edits":[{"id":1,"from":"確立変数","to":"確率変数"}]})
                                .to_string(),
                        )
                    } else {
                        response.clone()
                    })
                },
            )
            .await
            .unwrap();
            assert_eq!(parsed.corrected, "確率変数を定義します。");
            assert!(parsed.translation.is_none());
            assert!(parsed.sentences[0].translation.is_none());
            assert_eq!(parsed.terms.len(), 1);
        }
    }

    #[tokio::test]
    async fn fragment_boundaries_and_partially_rejected_translations_keep_their_ids() {
        let source = "まず確立変数\n次は60点以上なら合格です。";
        let mut calls = 0;
        let parsed = request_with(
            job(source),
            Deadline::now() + MODEL_BUDGET,
            |_, messages| {
                calls += 1;
                let response = if calls == 1 {
                    json!({"edits":[{"id":1,"from":"確立変数","to":"確率変数"}]})
                } else {
                    assert_eq!(input(&messages)["字幕"].as_array().unwrap().len(), 2);
                    json!({"translations":[
                        {"id":1,"text":"首先是随机变量"},
                        {"id":2,"text":"59分以上即可通过。"}
                    ]})
                };
                std::future::ready(Ok(response.to_string()))
            },
        )
        .await
        .unwrap();
        assert_eq!(parsed.sentences.len(), 2);
        assert_eq!(parsed.sentences[0].corrected, "まず確率変数");
        assert_eq!(
            parsed.sentences[0].translation.as_deref(),
            Some("首先是随机变量")
        );
        assert!(parsed.sentences[1].translation.is_none());
        assert!(parsed.translation.is_none());
    }

    #[tokio::test]
    async fn same_language_never_starts_a_translation_request() {
        let mut request = job("確立変数を定義します。");
        request.translate = false;
        request.target_language = "ja";
        let mut calls = 0;
        let parsed = request_with(request, Deadline::now() + MODEL_BUDGET, |_, _| {
            calls += 1;
            std::future::ready(Ok(
                json!({"edits":[{"id":1,"from":"確立変数","to":"確率変数"}]}).to_string(),
            ))
        })
        .await
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(parsed.corrected, "確率変数を定義します。");
        assert!(parsed.translation.is_none());
    }

    #[tokio::test]
    async fn correction_timeout_translates_only_the_retained_original() {
        let mut calls = 0;
        let parsed = request_with(
            job("確立変数を定義します。"),
            Deadline::now() + MODEL_BUDGET,
            |_, messages| {
                calls += 1;
                let call = calls;
                async move {
                    if call == 1 {
                        std::future::pending().await
                    } else {
                        assert_eq!(
                            input(&messages)["字幕"][0]["text"],
                            "確立変数を定義します。"
                        );
                        Ok(json!({"translations":[{"id":1,"text":"定义确立变量。"}]}).to_string())
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(parsed.corrected, "確立変数を定義します。");
        assert_eq!(parsed.translation.as_deref(), Some("定义确立变量。"));
        assert!(parsed.terms.is_empty());
    }

    #[tokio::test]
    async fn toggling_translation_off_cancels_the_second_stage() {
        let preference = CaptionPreference {
            enabled: true,
            revision: 0,
        };
        let (tx, rx) = watch::channel(preference);
        let (started, ready) = tokio::sync::oneshot::channel();
        let mut started = Some(started);
        let mut calls = 0;
        let work = request_with(
            job("確立変数を定義します。"),
            Deadline::now() + MODEL_BUDGET,
            |_, _| {
                calls += 1;
                let signal = if calls == 2 { started.take() } else { None };
                async move {
                    if let Some(signal) = signal {
                        signal.send(()).unwrap();
                        std::future::pending().await
                    } else {
                        Ok(
                            json!({"edits":[{"id":1,"from":"確立変数","to":"確率変数"}]})
                                .to_string(),
                        )
                    }
                }
            },
        );
        let (result, ()) = tokio::join!(until_preference_changes(rx, preference, work), async {
            ready.await.unwrap();
            tx.send_replace(CaptionPreference {
                enabled: false,
                revision: 1,
            });
        });
        assert!(result.is_none());
        assert_eq!(calls, 2);
    }

    #[tokio::test]
    async fn stages_share_one_deadline_and_timeout_retains_the_checked_source() {
        let start = Deadline::now();
        let deadline = start + Duration::from_millis(150);
        let mut calls = 0;
        let dropped = Arc::new(AtomicBool::new(false));
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let parsed = request_with(
            job("確立変数を定義します。"),
            deadline,
            |_, _| {
                calls += 1;
                let call = calls;
                let dropped = dropped.clone();
                async move {
                    if call == 1 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Ok(
                            json!({"edits":[{"id":1,"from":"確立変数","to":"確率変数"}]})
                                .to_string(),
                        )
                    } else {
                        let _guard = Dropped(dropped);
                        std::future::pending().await
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(calls, 2);
        assert!(start.elapsed() < Duration::from_millis(400));
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(parsed.corrected, "確率変数を定義します。");
        assert!(parsed.translation.is_none());
    }

    // Exercises the actual production two-call path and native guided schemas.
    // Kept opt-in because it requires an available Apple Intelligence model.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires Apple Intelligence on this machine"]
    async fn actual_apple_pipeline_translates_the_validated_source() {
        let source = "確立変数の期待地を求めます。";
        let mut request = job(source);
        request.lesson_terms = "確率変数\n期待値".into();
        let mut glossary = Glossary::default();
        glossary.learn("確立変数".into(), "確率変数".into());
        glossary.learn("期待地".into(), "期待値".into());
        request.glossary = glossary;
        let start = Deadline::now();
        let mut calls = Vec::new();
        let parsed = request_with(request, start + MODEL_BUDGET, |config, messages| {
            let value = input(&messages);
            eprintln!("production caption input: {value}");
            calls.push(value);
            async move {
                let raw = crate::ai::caption_completion(&config, messages).await;
                eprintln!("production caption result: {raw:?}");
                raw
            }
        })
        .await
        .unwrap();
        eprintln!(
            "pipeline elapsed: {:?}; accepted source: {}; translation: {:?}",
            start.elapsed(),
            parsed.corrected,
            parsed.translation
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1]["字幕"][0]["text"], parsed.sentences[0].corrected);
        assert_ne!(
            parsed.corrected, source,
            "the real model must correct at least one term in this smoke case"
        );
        assert!(
            parsed.translation.is_some(),
            "the real translator must return a usable translation"
        );
    }
}
