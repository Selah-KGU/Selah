use super::super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaTodoItem {
    pub course_name: String,
    pub content_type: String, // 課題, テスト, 掲示板
    pub content_name: String,
    pub url: String,
    pub deadline: String,
    pub status: String, // 未提出, 提出済み, etc.
    pub feedback: String,
}

pub fn parse_luna_todo(html: &str) -> Vec<LunaTodoItem> {
    let doc = Html::parse_document(html);
    let mut items = Vec::new();

    for item in doc.select(&SEL_TODO_LIST) {
        let course_name = item
            .select(&SEL_TODO_COURSE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let content_type = item
            .select(&SEL_TODO_TYPE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let (content_name, url) = item
            .select(&SEL_TODO_NAME)
            .next()
            .map(|e| {
                (
                    e.text().collect::<String>().trim().to_string(),
                    e.value().attr("href").unwrap_or_default().to_string(),
                )
            })
            .unwrap_or_default();

        let deadline = item
            .select(&SEL_TODO_DEADLINE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = item
            .select(&SEL_TODO_STATUS)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let feedback = item
            .select(&SEL_TODO_FEEDBACK)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaTodoItem {
            course_name,
            content_type,
            content_name,
            url,
            deadline,
            status,
            feedback,
        });
    }

    items
}
