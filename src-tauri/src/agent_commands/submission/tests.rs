use super::*;
use serde_json::json;

#[test]
fn document_only_input_is_admitted_and_keeps_the_complete_document_text() {
    let document = json!({"name":"資料.pdf","mime":"application/pdf","size":120,"text":"日本語の全文 🌕\n第二頁","truncated":false});
    let payload =
        InvokeBody::Json(json!({"convId":"A","content":"","images":[],"documents":[document]}));
    assert!(validate(&payload, false).is_ok());
    let (content, images, context) = decode(&payload, false, None).unwrap();
    assert!(content.is_empty());
    assert!(images.is_empty());
    assert_eq!(context.documents[0].text, "日本語の全文 🌕\n第二頁");
    for documents in [
        json!([{"name":"x.txt","mime":"text/plain","size":1,"text":""}]),
        json!([
            document.clone(),
            document.clone(),
            document.clone(),
            document.clone(),
            document
        ]),
    ] {
        assert!(validate(
            &InvokeBody::Json(json!({"convId":"A","content":"test","documents":documents})),
            false
        )
        .is_err());
    }
}

#[test]
fn admission_borrows_complete_strings_and_decoding_preserves_all_images_and_context() {
    let data = "A".repeat(2 * 1024 * 1024);
    let payload = InvokeBody::Json(json!({
        "convId": "A", "turnId": "request", "content": "　日本語\n全文　",
        "images": [{ "mime": "image/png", "data_base64": data }, { "mime": "image/jpeg", "data_base64": "BB==" }],
        "browserTarget": " course-view ", "pageTitle": " 授業 ", "pageKind": " luna "
    }));
    let (fields, context) = validate(&payload, true).unwrap();
    let InvokeBody::Json(value) = &payload else {
        unreachable!()
    };
    assert_eq!(
        fields.content.as_ptr(),
        value["content"].as_str().unwrap().as_ptr()
    );
    assert_eq!(
        fields.images.as_ref().unwrap()[0].data_base64.as_ptr(),
        value["images"][0]["data_base64"].as_str().unwrap().as_ptr()
    );
    assert_eq!(fields.turn_id, Some("request"));
    assert_eq!(nonempty(context.browser_target), Some("course-view"));
    let (text, images, context) = decode(&payload, true, Some("course-view-ct".into())).unwrap();
    assert_eq!(text, "日本語\n全文");
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].data_base64, data);
    assert_eq!(images[1].mime, "image/jpeg");
    assert_eq!(images[1].data_base64, "BB==");
    assert_eq!(context.browser_target.as_deref(), Some("course-view-ct"));
    assert_eq!(context.page_title.as_deref(), Some("授業"));
    assert_eq!(context.page_kind.as_deref(), Some("luna"));
}

#[test]
fn legacy_optional_fields_image_only_inputs_and_extra_fields_keep_existing_semantics() {
    for extra in [
        json!({}),
        json!({"turnId": null, "images": null}),
        json!({"images": []}),
    ] {
        let mut value = json!({ "convId": "A", "content": "hello" });
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let payload = InvokeBody::Json(value);
        let (fields, _) = validate(&payload, false).unwrap();
        assert!(fields.turn_id.is_none());
        let (text, images, _) = decode(&payload, false, None).unwrap();
        assert_eq!(text, "hello");
        assert!(images.is_empty());
    }
    let payload = InvokeBody::Json(
        json!({ "convId": "A", "content": "　", "images": [{ "mime":"image/png", "data_base64":"AA==" }], "pageTitle":false }),
    );
    assert!(validate(&payload, false).is_ok()); // Plain send ignores page fields.
    assert!(validate(&payload, true).is_err());
    let (text, images, _) = decode(&payload, false, None).unwrap();
    assert!(text.is_empty());
    assert_eq!(images.len(), 1);
    let payload = InvokeBody::Json(
        json!({ "convId": "A", "content": "hi", "browserTarget": "　", "pageTitle": null, "pageKind": "　" }),
    );
    let (_, context) = validate(&payload, true).unwrap();
    assert!(nonempty(context.browser_target).is_none());
    let (_, _, context) = decode(&payload, true, None).unwrap();
    assert!(context.page_title.is_none());
    assert!(context.page_kind.is_none());
}

#[test]
fn invalid_payloads_are_rejected_before_admission_can_replace_a_live_turn() {
    for value in [
        json!({ "convId":"A" }),
        json!({ "content":"hi" }),
        json!({ "convId":false, "content":"hi" }),
        json!({ "convId":"A", "content":[] }),
        json!({ "convId":"A", "content":"　", "images":[] }),
        json!({ "convId":"A", "content":"hi", "turnId":7 }),
        json!({ "convId":"A", "content":"hi", "images":{} }),
        json!({ "convId":"A", "content":"hi", "images":[{ "mime":"image/png" }] }),
        json!({ "convId":"A", "content":"hi", "images":[{ "mime":"image/png", "data_base64":null }] }),
        json!({ "convId":"A", "content":"hi", "browserTarget":false }),
    ] {
        assert!(validate(&InvokeBody::Json(value), true).is_err());
    }
    assert!(validate(&InvokeBody::Raw(vec![]), false).is_err());
}

#[test]
fn invalid_context_and_closed_save_gate_do_not_admit_or_replace_the_existing_request() {
    let id = uuid::Uuid::new_v4().to_string();
    let running = crate::agent_turn_scope::RunningTurn::begin(&id, Some("existing".into()));
    let saves = std::sync::Arc::new(crate::pending_persistence::PendingPersistence::default());
    let invalid =
        InvokeBody::Json(json!({ "convId":id, "content":"hi", "images":[{"mime":"image/png"}] }));
    assert!(prepare_control(
        &invalid,
        true,
        |_| panic!("invalid args resolved a target"),
        &saves
    )
    .is_err());
    assert!(!running.turn.cancelled());
    let missing =
        InvokeBody::Json(json!({ "convId":id, "content":"hi", "browserTarget":"missing" }));
    assert!(prepare_control(&missing, true, |_| Err("not found".into()), &saves).is_err());
    assert!(!running.turn.cancelled());
    saves.seal();
    let valid = InvokeBody::Json(json!({ "convId":id, "content":"hi" }));
    assert!(prepare_control(
        &valid,
        false,
        |_| panic!("plain send resolved a target"),
        &saves
    )
    .is_err());
    assert!(!running.turn.cancelled());
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(3), saves.drained())
            .await
            .unwrap()
            .unwrap();
    });
}
