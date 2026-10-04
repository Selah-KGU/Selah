//! Open a known Copilot page from an agent tool call.

use super::*;

pub(super) enum LunaCopilotDestination {
    Detail(String),
    External(String),
}

pub(super) fn luna_path_query(path: &str, key: &str) -> Option<String> {
    let full = if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{}{}", crate::config::LUNA_BASE, path)
    };
    url::Url::parse(&full)
        .ok()?
        .query_pairs()
        .find_map(|(k, v)| (k == key).then(|| v.into_owned()))
        .filter(|value| !value.is_empty())
}

pub(super) fn luna_activity_copilot_destination(
    row: &crate::db::LunaActivityRow,
) -> LunaCopilotDestination {
    let encoded_path = urlencoding::encode(&row.detail_path);
    let encoded_title = urlencoding::encode(&row.title);
    let encoded_id = urlencoding::encode(&row.luna_id);
    let encoded_period = urlencoding::encode(&row.period);
    let encoded_status = urlencoding::encode(&row.status);
    let params = match row.activity_type.as_str() {
        "announcement" => luna_path_query(&row.detail_path, "informationId")
            .map(|info_id| {
                format!(
                    "mode=announcement&title={}&idnumber={}&infoId={}",
                    encoded_title,
                    encoded_id,
                    urlencoding::encode(&info_id),
                )
            })
            .unwrap_or_else(|| format!("path={}&title={}", encoded_path, encoded_title)),
        "report" => {
            let report_id = luna_path_query(&row.detail_path, "reportId").unwrap_or_default();
            format!(
                "mode=report&path={}&title={}&idnumber={}&reportId={}&period={}",
                encoded_path,
                encoded_title,
                encoded_id,
                urlencoding::encode(&report_id),
                encoded_period,
            )
        }
        "discussion" => format!(
            "mode=discussion&path={}&title={}",
            encoded_path, encoded_title
        ),
        "exam" => {
            let url = if row.detail_path.starts_with("http://")
                || row.detail_path.starts_with("https://")
            {
                row.detail_path.clone()
            } else {
                format!("{}{}", crate::config::LUNA_BASE, row.detail_path)
            };
            return LunaCopilotDestination::External(url);
        }
        _ => format!(
            "path={}&title={}&idnumber={}&period={}&status={}",
            encoded_path, encoded_title, encoded_id, encoded_period, encoded_status,
        ),
    };
    LunaCopilotDestination::Detail(params)
}

pub(super) async fn open_copilot_page(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let page = args
        .get("page")
        .and_then(|v| v.as_str())
        .ok_or("pageは必須です")?;
    let context = args
        .get("context")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let luna_id = args
        .get("luna_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let identifier = args
        .get("identifier")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Whether the Copilot window already existed before this call. If not, this
    // call is what opens it (e.g. the main-window agent opening a Copilot page),
    // so afterwards we also bring up the sidebar agent for a continuous chat.
    let copilot_window_existed = app.get_window("document-tabs").is_some();

    let (title, target) = match page {
        "new_tab" => {
            let tab = crate::document_tabs::open_new_tab(app)?;
            (tab.title, tab.target)
        }
        "files" => {
            let tab = crate::document_tabs::open_files_tab(
                app,
                context.map(str::to_string),
                "ファイル".to_string(),
            )?;
            (tab.title, tab.target)
        }
        "luna" => {
            let tab = crate::document_tabs::open_external_tab(
                app,
                crate::config::LUNA_BASE.to_string(),
                Some("Luna".to_string()),
            )?;
            (tab.title, tab.target)
        }
        "luna_course" => {
            let query = context.ok_or("luna_courseにはcontext(科目名)が必要です")?;
            let needle = normalize_text(query);
            let db = app.state::<Database>();
            let courses = db.get_luna_courses().unwrap_or_default();
            let candidates = courses
                .iter()
                .filter(|row| luna_id.is_none_or(|id| row.luna_id == id))
                .collect::<Vec<_>>();
            let row = candidates
                .iter()
                .find(|row| normalize_text(&row.name) == needle)
                .or_else(|| {
                    candidates.iter().find(|row| {
                        let name = normalize_text(&row.name);
                        name.contains(&needle) || needle.contains(&name)
                    })
                })
                .copied()
                .ok_or_else(|| format!("「{}」に一致する科目が見つかりません", query))?;
            // Luna course-top detail page; the course detail surface uses the
            // course's luna_id as its idnumber (see Timetable/HomePage).
            let params = format!(
                "mode=course&idnumber={}&title={}",
                urlencoding::encode(&row.luna_id),
                urlencoding::encode(&row.name),
            );
            let tab =
                crate::document_tabs::open_university_detail_tab(app, params, row.name.clone())?;
            (tab.title, tab.target)
        }
        "luna_activity" => {
            let query = context.ok_or("luna_activityにはcontextが必要です")?;
            let needle = normalize_text(query);
            let db = app.state::<Database>();
            let activities = db.get_all_luna_activities().unwrap_or_default();
            let candidates = activities
                .iter()
                .filter(|row| luna_id.is_none_or(|id| row.luna_id == id))
                .collect::<Vec<_>>();
            let row = candidates
                .iter()
                .find(|row| normalize_text(&row.title) == needle)
                .or_else(|| {
                    candidates.iter().find(|row| {
                        let title = normalize_text(&row.title);
                        title.contains(&needle) || needle.contains(&title)
                    })
                })
                .copied()
                .ok_or_else(|| format!("「{}」に一致するLuna活動が見つかりません", query))?;
            if row.detail_path.is_empty() {
                return Err(format!("「{}」には詳細ページがありません", row.title));
            }
            match luna_activity_copilot_destination(row) {
                LunaCopilotDestination::Detail(params) => {
                    let tab = crate::document_tabs::open_university_detail_tab(
                        app,
                        params,
                        row.title.clone(),
                    )?;
                    (tab.title, tab.target)
                }
                LunaCopilotDestination::External(url) => {
                    let tab =
                        crate::document_tabs::open_external_tab(app, url, Some(row.title.clone()))?;
                    (tab.title, tab.target)
                }
            }
        }
        "kwic" => {
            let tab = crate::document_tabs::open_external_tab(
                app,
                crate::config::KWIC_BASE.to_string(),
                Some("KWIC".to_string()),
            )?;
            (tab.title, tab.target)
        }
        "kwic_notification" => {
            let needle = context.map(normalize_text);
            let db = app.state::<Database>();
            let (json_str, _) = db
                .get_data_cache("kwic_home")
                .map_err(|e| e.to_string())?
                .ok_or("KWIC通知キャッシュがありません。先に通知を取得してください")?;
            let home: crate::kwic_commands::KwicPortalHome =
                serde_json::from_str(&json_str).map_err(|e| e.to_string())?;
            let item = home
                .sections
                .iter()
                .flat_map(|section| section.items.iter())
                .find(|item| identifier.is_some_and(|id| item.id == id))
                .or_else(|| {
                    home.sections
                        .iter()
                        .flat_map(|section| section.items.iter())
                        .find(|item| {
                            needle.as_ref().is_some_and(|needle| {
                                let title = normalize_text(&item.title);
                                title == needle.as_str()
                                    || title.contains(needle)
                                    || needle.contains(&title)
                            })
                        })
                })
                .ok_or_else(|| {
                    format!(
                        "指定されたKWIC通知が見つかりません: {}",
                        context.or(identifier).unwrap_or("")
                    )
                })?;
            if item.id.is_empty() {
                return Err(format!("「{}」には詳細IDがありません", item.title));
            }
            let params = format!(
                "mode=kwic&informationId={}&informationType={}&personCategoryCd={}&categoryCd={}&title={}",
                urlencoding::encode(&item.id),
                urlencoding::encode(&item.information_type),
                urlencoding::encode(&item.person_category_cd),
                urlencoding::encode(&item.category_cd),
                urlencoding::encode(&item.title),
            );
            let tab =
                crate::document_tabs::open_university_detail_tab(app, params, item.title.clone())?;
            (tab.title, tab.target)
        }
        "kwic_cabinet" => {
            let title = context.unwrap_or("学生キャビネット");
            let params = format!("mode=kwicCabinet&title={}", urlencoding::encode(title));
            let tab =
                crate::document_tabs::open_university_detail_tab(app, params, title.to_string())?;
            (tab.title, tab.target)
        }
        "kgc" => {
            let tab = crate::document_tabs::open_external_tab(
                app,
                crate::config::KG_COURSE_BASE.to_string(),
                Some("KG Course".to_string()),
            )?;
            (tab.title, tab.target)
        }
        "kgc_notification" => {
            let needle = context.map(normalize_text);
            let db = app.state::<Database>();
            let (json_str, _) = db
                .get_data_cache("notifications")
                .map_err(|e| e.to_string())?
                .ok_or("KGC通知キャッシュがありません。先に通知を取得してください")?;
            let data: crate::parser::NotificationsData =
                serde_json::from_str(&json_str).map_err(|e| e.to_string())?;
            let item = data
                .entries
                .iter()
                .find(|item| identifier.is_some_and(|id| item.id == id))
                .or_else(|| {
                    data.entries.iter().find(|item| {
                        needle.as_ref().is_some_and(|needle| {
                            let title = normalize_text(&item.title);
                            title == needle.as_str()
                                || title.contains(needle)
                                || needle.contains(&title)
                        })
                    })
                })
                .ok_or_else(|| {
                    format!(
                        "指定されたKGC通知が見つかりません: {}",
                        context.or(identifier).unwrap_or("")
                    )
                })?;
            if item.url.is_empty() {
                return Err(format!("「{}」には詳細ページがありません", item.title));
            }
            let path = if item.url.starts_with('/') {
                item.url.clone()
            } else if item.url.starts_with("http") {
                return Err("KGC外部リンクはCopilot詳細ページで開けません".into());
            } else {
                format!("/uniasv2/{}", item.url)
            };
            let params = format!(
                "mode=kgc&path={}&name={}",
                urlencoding::encode(&path),
                urlencoding::encode(&item.title),
            );
            let tab =
                crate::document_tabs::open_university_detail_tab(app, params, item.title.clone())?;
            (tab.title, tab.target)
        }
        _ => return Err(format!("未対応のCopilotページです: {}", page)),
    };

    // Newly opening the Copilot window (e.g. from the main-window agent): also
    // reveal the sidebar agent so the conversation can continue there.
    if !copilot_window_existed {
        let _ = crate::document_tabs::open_agent_workspace(app);
    }

    Ok(json!({
        "page": page,
        "title": title,
        "target": target,
        "opened_in": "copilot",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luna_copilot_destination_uses_activity_specific_surface() {
        let row = |activity_type: &str, detail_path: &str| crate::db::LunaActivityRow {
            luna_id: "LUNA-42".into(),
            activity_type: activity_type.into(),
            title: "第7回課題".into(),
            period: "2026-06-20".into(),
            status: "未提出".into(),
            detail_path: detail_path.into(),
        };

        let LunaCopilotDestination::Detail(announcement) = luna_activity_copilot_destination(&row(
            "announcement",
            "/lms/coursetop/information/listdetail?idnumber=LUNA-42&informationId=7",
        )) else {
            panic!("announcement should use detail surface");
        };
        assert!(announcement.starts_with("mode=announcement&"));
        assert!(announcement.contains("infoId=7"));

        let LunaCopilotDestination::Detail(report) = luna_activity_copilot_destination(&row(
            "report",
            "/lms/course/report/submission?idnumber=LUNA-42&reportId=9",
        )) else {
            panic!("report should use detail surface");
        };
        assert!(report.starts_with("mode=report&"));
        assert!(report.contains("reportId=9"));

        let LunaCopilotDestination::Detail(discussion) = luna_activity_copilot_destination(&row(
            "discussion",
            "/lms/course/forums/themetop?forumId=3",
        )) else {
            panic!("discussion should use detail surface");
        };
        assert!(discussion.starts_with("mode=discussion&"));

        let LunaCopilotDestination::External(exam) =
            luna_activity_copilot_destination(&row("exam", "/lms/course/exam/start?id=5"))
        else {
            panic!("exam should use external Luna page");
        };
        assert_eq!(
            exam,
            "https://luna.kwansei.ac.jp/lms/course/exam/start?id=5"
        );
    }
}
