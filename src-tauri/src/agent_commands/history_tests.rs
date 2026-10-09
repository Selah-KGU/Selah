use super::*;

#[test]
fn attachment_migration_preserves_an_existing_legacy_conversation() {
    let temporary = Temporary::new();
    std::fs::create_dir_all(&temporary.0).unwrap();
    let connection = rusqlite::Connection::open(temporary.0.join("courses.db")).unwrap();
    connection.execute_batch("CREATE TABLE agent_conversations(id TEXT PRIMARY KEY,title TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
        CREATE TABLE agent_messages(id INTEGER PRIMARY KEY AUTOINCREMENT,conv_id TEXT NOT NULL,role TEXT NOT NULL,content TEXT NOT NULL,images_json TEXT,tool_name TEXT,tool_result_json TEXT,created_at INTEGER NOT NULL);
        INSERT INTO agent_conversations VALUES('c','legacy',1,1);
        INSERT INTO agent_messages(conv_id,role,content,created_at) VALUES('c','user','既存の全文 🌕',1);").unwrap();
    drop(connection);
    let db = Database::open(&temporary.0).unwrap();
    let rows = load_display_messages(&db, "c").unwrap();
    assert_eq!(rows[0].content, "既存の全文 🌕");
    assert!(rows[0].documents.is_empty());
    assert!(db.agent_load_message_documents("c").unwrap().is_empty());
}

#[test]
fn document_history_restores_the_original_draft_and_typed_attachment_after_reopening() {
    let temporary = Temporary::new();
    let document = crate::agent_attachments::DocumentPart {
        name: "資料.docx".into(),
        mime: "application/docx".into(),
        size: 4,
        text: "日本語の資料 🌕".into(),
        truncated: false,
    };
    {
        let db = Database::open(&temporary.0).unwrap();
        db.agent_create_conversation("c", "documents").unwrap();
        let saved = crate::agent_attachments::SavedDocuments {
            content: "説明して".into(),
            documents: vec![document.clone()],
        };
        db.agent_append_document_message(
            "c",
            &crate::agent_attachments::model_content(&saved.content, &saved.documents),
            None,
            &serde_json::to_string(&saved).unwrap(),
        )
        .unwrap();
        assert!(db.agent_load_messages("c").unwrap()[0]
            .content
            .contains("日本語の資料 🌕"));
    }
    let db = Database::open(&temporary.0).unwrap();
    let rows = load_display_messages(&db, "c").unwrap();
    assert_eq!(rows[0].content, "説明して");
    assert_eq!(rows[0].documents[0].text, document.text);
    assert_eq!(rows[0].documents[0].name, document.name);
}

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("selah-display-dto-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn display_dto_preserves_visible_text_metadata_and_attachment_decoding() {
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "c").unwrap();
    for (role, images) in [
        (
            "user",
            Some(r#"[{"mime":"image/png","data_base64":"あ🌕AA=="}]"#),
        ),
        (
            "assistant",
            Some(r#"[{"mime":"image/jpeg","data_base64":"BB=="}]"#),
        ),
        ("user", Some("broken json")),
        ("user", Some("[]")),
        ("assistant", None),
    ] {
        db.agent_append_message("c", role, "日本語の全文 🌕", images, None, None)
            .unwrap();
        db.agent_append_message(
            "c",
            "tool",
            "",
            None,
            Some("screenshot"),
            Some(r#"{"image":{"mime":"image/png","data_base64":"unused"}}"#),
        )
        .unwrap();
    }
    let expected = db
        .agent_load_messages("c")
        .unwrap()
        .into_iter()
        .filter(|row| row.role == "user" || row.role == "assistant")
        .map(AgentMessageDto::from)
        .collect::<Vec<_>>();
    let display = load_display_messages(&db, "c").unwrap();
    assert_eq!(
        serde_json::to_value(&display).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(
        display[0].images.as_ref().unwrap()[0].data_base64,
        "あ🌕AA=="
    );
    assert!(display[2].images.is_none());
    assert_eq!(display[3].images.as_ref().unwrap().len(), 0);
    use tauri::ipc::{InvokeResponseBody, IpcResponse};
    let body = load_display_response(&db, "c").unwrap().body().unwrap();
    assert!(matches!(&body, InvokeResponseBody::Json(json) if json.starts_with('[')));
    let decoded = body.deserialize::<Vec<AgentMessageDto>>().unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::to_value(&display).unwrap()
    );
}

#[test]
fn full_history_response_keeps_the_legacy_array_and_every_tool_and_attachment() {
    use tauri::ipc::{InvokeResponseBody, IpcResponse};
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "c").unwrap();
    let image =
        serde_json::json!({"image":{"mime":"image/png","data_base64":"あ🌕".repeat(20_000)}})
            .to_string();
    for (role, attachment, result) in [
        (
            "user",
            Some(r#"[{"mime":"image/jpeg","data_base64":"full attachment"}]"#),
            None,
        ),
        ("tool", None, Some(image.as_str())),
        ("tool", Some("broken attachment"), Some("broken result")),
        ("assistant", None, None),
    ] {
        db.agent_append_message(
            "c",
            role,
            "full text 🌕",
            attachment,
            Some("example"),
            result,
        )
        .unwrap();
    }
    let expected = db
        .agent_load_messages("c")
        .unwrap()
        .into_iter()
        .map(AgentMessageDto::from)
        .collect::<Vec<_>>();
    let body = load_full_response(&db, "c").unwrap().body().unwrap();
    assert!(matches!(&body, InvokeResponseBody::Json(json) if json.starts_with('[')));
    let actual = body.deserialize::<Vec<AgentMessageDto>>().unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert!(load_full_response(&db, "missing")
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<Vec<AgentMessageDto>>()
        .unwrap()
        .is_empty());
    let conn = rusqlite::Connection::open(temporary.0.join("courses.db")).unwrap();
    conn.execute(
        "UPDATE agent_messages SET content=x'ff' WHERE role='tool'",
        [],
    )
    .unwrap();
    assert!(load_full_response(&db, "c")
        .err()
        .unwrap()
        .starts_with("DB read message:"));
}

#[test]
#[ignore = "manual query/DTO/JSON comparison, not WebKit CPU or GPU performance"]
fn benchmark_agent_display_history() {
    use rusqlite::{params, Connection};
    let temporary = Temporary::new();
    let db = Database::open(&temporary.0).unwrap();
    for id in ["tool-heavy", "text-only"] {
        db.agent_create_conversation(id, id).unwrap();
    }
    let large_image =
        serde_json::json!({"image":{"mime":"image/png","data_base64":"A".repeat(2*1024*1024)}})
            .to_string();
    let body = serde_json::json!({"body":"あ🌕 text ".repeat(1024)}).to_string();
    let attachment = r#"[{"mime":"image/png","data_base64":"AA=="}]"#;
    {
        let mut conn = Connection::open(temporary.0.join("courses.db")).unwrap();
        let tx = conn.transaction().unwrap();
        for n in 0..2_000 {
            let role = if n % 10 == 0 {
                "user"
            } else if n % 10 == 1 {
                "assistant"
            } else {
                "tool"
            };
            tx.execute("INSERT INTO agent_messages(conv_id,role,content,images_json,tool_name,tool_result_json,created_at) VALUES ('tool-heavy',?1,?2,?3,?4,?5,?6)",
                params![role, if role=="tool" { "" } else { "日本語 visible text" }, if role=="user" {Some(attachment)} else {None},
                    if role=="tool" {Some("example")} else {None}, if role=="tool" {Some(if n==1999 {&large_image} else {&body})} else {None}, n/10]).unwrap();
            if role != "tool" {
                tx.execute("INSERT INTO agent_messages(conv_id,role,content,images_json,created_at) VALUES ('text-only',?1,'日本語 visible text',?2,?3)", params![role, if role=="user" {Some(attachment)} else {None}, n/10]).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    for id in ["tool-heavy", "text-only"] {
        let mut old_times = Vec::new();
        let mut new_times = Vec::new();
        let mut sizes = (0, 0);
        for round in 0..8 {
            for display in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let start = std::time::Instant::now();
                let json = if display {
                    use tauri::ipc::{InvokeResponseBody, IpcResponse};
                    match load_display_response(&db, id).unwrap().body().unwrap() {
                        InvokeResponseBody::Json(json) => json,
                        InvokeResponseBody::Raw(_) => panic!("display response must remain JSON"),
                    }
                } else {
                    let rows = db
                        .agent_load_messages(id)
                        .unwrap()
                        .into_iter()
                        .map(AgentMessageDto::from)
                        .collect::<Vec<_>>();
                    serde_json::to_string(&rows).unwrap()
                };
                let duration = start.elapsed();
                if display {
                    sizes.1 = json.len()
                } else {
                    sizes.0 = json.len()
                }
                if round > 0 {
                    if display {
                        new_times.push(duration)
                    } else {
                        old_times.push(duration)
                    }
                }
                std::hint::black_box(&json);
            }
        }
        old_times.sort();
        new_times.sort();
        let expected = db
            .agent_load_messages(id)
            .unwrap()
            .into_iter()
            .filter(|row| row.role == "user" || row.role == "assistant")
            .map(AgentMessageDto::from)
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::to_value(load_display_messages(&db, id).unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        println!("{id}: query + production DTO + JSON, debug build, 7 alternating samples; full {:?}, display {:?}; IPC JSON {} -> {} bytes; visible rows 400 retained",old_times[3],new_times[3],sizes.0,sizes.1);
    }
}
