use super::super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaNotification {
    pub date: String,
    pub course_info: String,
    pub module: String, // 掲示板スレッド, お知らせ, 課題, etc.
    pub content: String,
    pub url: String,
    pub idnumber: String,
}

pub fn parse_luna_notifications(html: &str) -> Vec<LunaNotification> {
    let doc = Html::parse_document(html);
    let mut items = Vec::new();

    for item in doc.select(&SEL_NOTIF_LIST) {
        let date = item
            .select(&SEL_NOTIF_DATE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let course_info = item
            .select(&SEL_NOTIF_COURSE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let module = item
            .select(&SEL_NOTIF_MODULE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let content = item
            .select(&SEL_NOTIF_CONTENT)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let url = item
            .select(&SEL_NOTIF_URL)
            .next()
            .and_then(|e| e.value().attr("value"))
            .unwrap_or_default()
            .to_string();

        let idnumber = item
            .select(&SEL_INPUT_IDNUMBER)
            .next()
            .or_else(|| item.select(&SEL_INPUT_IDNAME).next())
            .and_then(|e| e.value().attr("value"))
            .unwrap_or_default()
            .to_string();

        if date.is_empty() {
            continue;
        }

        // Skip LUNA system-wide announcements (e.g. 時間割 section notices about
        // guest access, maintenance schedules, etc.) — these are not course-specific
        // and cause errors when detail-fetched.
        if course_info == "時間割" {
            continue;
        }

        items.push(LunaNotification {
            date,
            course_info,
            module,
            content,
            url,
            idnumber,
        });
    }

    items
}
