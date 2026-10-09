use super::*;

#[test]
fn document_input_persists_model_text_and_display_metadata_in_one_transaction() {
    let root = std::env::temp_dir().join(format!("selah-document-turn-{}", uuid::Uuid::new_v4()));
    let db = Database::open(&root).unwrap();
    db.agent_create_conversation("c", "documents").unwrap();
    let images = vec![ImagePart {
        mime: "image/png".into(),
        data_base64: "AA==".into(),
    }];
    let documents = vec![crate::agent_attachments::DocumentPart {
        name: "資料.pdf".into(),
        mime: "application/pdf".into(),
        size: 100,
        text: "日本語の内容 🌕".into(),
        truncated: true,
    }];
    let id = persist_user_documents(&db, "c", "要約して", &images, &documents).unwrap();
    let history = db.agent_load_messages("c").unwrap();
    assert_eq!(history[0].id, id);
    assert_eq!(
        history[0].content,
        crate::agent_attachments::model_content("要約して", &documents)
    );
    assert_eq!(
        history[0].images_json.as_deref(),
        Some(serde_json::to_string(&images).unwrap().as_str())
    );
    let saved: crate::agent_attachments::SavedDocuments =
        serde_json::from_str(&db.agent_load_message_documents("c").unwrap()[&id]).unwrap();
    assert_eq!(saved.content, "要約して");
    assert_eq!(saved.documents[0].text, documents[0].text);
    let connection = rusqlite::Connection::open(root.join("courses.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_document BEFORE INSERT ON agent_messages WHEN NEW.documents_json IS NOT NULL BEGIN SELECT RAISE(ABORT,'document write failed'); END;").unwrap();
    assert!(persist_user_documents(&db, "c", "second", &[], &documents).is_err());
    assert_eq!(db.agent_load_messages("c").unwrap().len(), 1);
    drop(connection);
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}
