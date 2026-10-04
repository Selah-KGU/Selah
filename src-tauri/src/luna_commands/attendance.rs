//! Luna attendance prefetch and submit.

use super::*;

macro_rules! sel {
    ($name:ident, $s:expr) => {
        static $name: LazyLock<scraper::Selector> =
            LazyLock::new(|| scraper::Selector::parse($s).expect(concat!("bad selector: ", $s)));
    };
}

/// Prefetch the attendance send form to extract time-window metadata
/// (送信可能日時, 遅刻時間, 内容) without submitting.
#[tauri::command]
pub async fn luna_prefetch_attendance_form(
    state: State<'_, LunaState>,
    idnumber: String,
    attendance_id: String,
) -> Result<serde_json::Value, String> {
    if !is_safe_param(&idnumber) || !is_safe_param(&attendance_id) {
        return Err("無効なパラメータです".into());
    }

    let http = luna_http(&state).await?;
    let send_path = format!(
        "/lms/course/attendances/send?idnumber={}&attendanceId={}",
        idnumber, attendance_id
    );
    let referer_path = format!("/lms/course?idnumber={}#attendance", idnumber);

    let html = match luna_get_with_referer(&http, &send_path, &referer_path).await {
        Ok(body) => body,
        Err(_) => luna_get(&http, &send_path).await?,
    };

    if html.contains("登録期間外") {
        return Err("登録期間外です".into());
    }
    if html.contains("登録済") || html.contains("出席済") {
        return Ok(serde_json::json!({ "already_registered": true }));
    }

    // Parse the contents-detail blocks to extract time info
    let doc = scraper::Html::parse_document(&html);
    sel!(SEL_DETAIL_BLOCK, ".contents-detail.contents-vertical");
    sel!(SEL_HEADER_TXT, ".contents-header.contents-header-txt");
    sel!(SEL_INPUT_AREA, ".contents-input-area");

    let mut open_start = String::new();
    let mut open_end = String::new();
    let mut late_start = String::new();
    let mut late_end = String::new();
    let mut content_text = String::new();

    for block in doc.select(&SEL_DETAIL_BLOCK) {
        let header_text = block
            .select(&SEL_HEADER_TXT)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let spans: Vec<String> = block
            .select(&SEL_INPUT_AREA)
            .next()
            .map(|area| {
                area.children()
                    .filter_map(|n| {
                        n.value().as_element().and_then(|e| {
                            if e.name() == "span" {
                                Some(
                                    scraper::ElementRef::wrap(n)
                                        .map(|er| er.text().collect::<String>().trim().to_string())
                                        .unwrap_or_default(),
                                )
                            } else {
                                None
                            }
                        })
                    })
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        if header_text.contains("送信可能日時") || header_text.contains("ログイン期間")
        {
            if !spans.is_empty() {
                open_start = spans[0].clone();
            }
            if spans.len() >= 3 {
                open_end = spans[2].clone();
            }
        } else if header_text.contains("遅刻時間") {
            if !spans.is_empty() {
                late_start = spans[0].clone();
            }
            if spans.len() >= 3 {
                late_end = spans[2].clone();
            }
        } else if header_text.contains("内容") && !header_text.contains("パスワード") {
            let area_text = block
                .select(&SEL_INPUT_AREA)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if !area_text.is_empty() {
                content_text = area_text;
            }
        }
    }

    Ok(serde_json::json!({
        "already_registered": false,
        "open_start": open_start,
        "open_end": open_end,
        "late_start": late_start,
        "late_end": late_end,
        "content": content_text,
    }))
}

/// Submit attendance registration (出席登録)
/// Flow: GET send page -> parse hidden form -> POST submit (up to 2 rounds)
#[tauri::command]
pub async fn luna_submit_attendance(
    state: State<'_, LunaState>,
    idnumber: String,
    attendance_id: String,
    one_time_pass: Option<String>,
    comment: Option<String>,
) -> Result<String, String> {
    if !is_safe_param(&idnumber) || !is_safe_param(&attendance_id) {
        return Err("無効なパラメータです".into());
    }

    let http = luna_http(&state).await?;
    let send_path = format!(
        "/lms/course/attendances/send?idnumber={}&attendanceId={}",
        idnumber, attendance_id
    );
    let referer_path = format!("/lms/course?idnumber={}#attendance", idnumber);

    let mut html = match luna_get_with_referer(&http, &send_path, &referer_path).await {
        Ok(body) => body,
        Err(_) => luna_get(&http, &send_path).await?,
    };

    if html.contains("登録期間外") {
        return Err("登録期間外です".into());
    }
    if html.contains("登録済") || html.contains("出席済") {
        return Ok("すでに登録済みです".into());
    }

    for _ in 0..2 {
        if html.contains("完了") || html.contains("登録しました") || html.contains("登録済")
        {
            return Ok("出席を登録しました".into());
        }

        let (action, mut fields) = match extract_form_fields(&html, "/attendances") {
            Some(v) => v,
            None => break,
        };
        if fields.is_empty() {
            break;
        }

        if let Some(pass) = one_time_pass.as_ref() {
            upsert_field(&mut fields, "oneTimePass", pass.clone());
        }
        if let Some(cmt) = comment.as_ref() {
            upsert_field(&mut fields, "comment", cmt.clone());
        }

        if let Some(current_pass) = field_value(&fields, "oneTimePass") {
            if current_pass.trim().is_empty() {
                return Err("出席パスワードを入力してください".into());
            }
        }

        let submit_url = if action.starts_with("http") {
            action.clone()
        } else {
            format!("{}{}", config::LUNA_BASE, action)
        };

        let referer = format!("{}{}", config::LUNA_BASE, send_path);
        let builder = http
            .post(&submit_url)
            .header("Referer", &referer)
            .form(&fields);
        html = client::send_and_follow_redirect(
            &http,
            builder,
            config::LUNA_BASE,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        )
        .await?;
    }

    if html.contains("完了") || html.contains("登録しました") || html.contains("登録済")
    {
        Ok("出席を登録しました".into())
    } else if html.contains("登録期間外") {
        Err("登録期間外です".into())
    } else {
        Err("出席登録フォームを完了できませんでした".into())
    }
}
