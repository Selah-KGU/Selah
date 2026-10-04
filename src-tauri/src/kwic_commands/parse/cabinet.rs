use crate::config;

use super::super::types::{KwicCabinetItem, KwicCabinetReference};
use super::information::normalize_text;
use super::{SEL_CABINET_DATE, SEL_CABINET_NEW, SEL_CABINET_ROW, SEL_CABINET_TITLE};

fn hidden_value(row: &scraper::ElementRef, class_name: &str) -> String {
    let selector = match scraper::Selector::parse(&format!("input.{}", class_name)) {
        Ok(sel) => sel,
        Err(_) => return String::new(),
    };
    row.select(&selector)
        .next()
        .and_then(|el| el.value().attr("value"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn absolute_kwic_url(path_or_url: &str) -> String {
    if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
        path_or_url.to_string()
    } else if path_or_url.starts_with('/') {
        format!("{}{}", config::KWIC_BASE, path_or_url)
    } else {
        format!("{}/{}", config::KWIC_BASE, path_or_url)
    }
}

fn cabinet_direct_url(list_url: &str, cabinet_id: &str) -> String {
    let base_path = if list_url.trim().is_empty() {
        "/cabinet/reference?typeCd=0"
    } else {
        list_url.trim()
    };
    let absolute = absolute_kwic_url(base_path);
    match url::Url::parse(&absolute) {
        Ok(mut url) => {
            let has_cabinet = url.query_pairs().any(|(key, _)| key == "cabinetId");
            let has_direct = url.query_pairs().any(|(key, _)| key == "directLink");
            {
                let mut pairs = url.query_pairs_mut();
                if !has_cabinet && !cabinet_id.is_empty() {
                    pairs.append_pair("cabinetId", cabinet_id);
                }
                if !has_direct {
                    pairs.append_pair("directLink", "1");
                }
            }
            url.to_string()
        }
        Err(_) => absolute,
    }
}

pub(in crate::kwic_commands) fn parse_cabinet_reference(html: &str) -> KwicCabinetReference {
    use scraper::Html;
    let doc = Html::parse_document(html);
    let mut items = Vec::new();

    for row in doc.select(&SEL_CABINET_ROW) {
        let cabinet_id = hidden_value(&row, "listCabinetId");
        let mut name = hidden_value(&row, "listCabinetName");
        let level = hidden_value(&row, "listCabinetLevel")
            .parse::<u32>()
            .unwrap_or(0);
        let list_url = hidden_value(&row, "listUrl");

        if name.is_empty() {
            name = row
                .select(&SEL_CABINET_TITLE)
                .next()
                .map(|el| normalize_text(&el.text().collect::<Vec<_>>().join(" ")))
                .unwrap_or_default();
        }
        if name.is_empty() || cabinet_id.is_empty() {
            continue;
        }

        let updated_at = row
            .select(&SEL_CABINET_NEW)
            .next()
            .and_then(|el| el.value().attr("data-value"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                row.select(&SEL_CABINET_DATE)
                    .next()
                    .map(|el| normalize_text(&el.text().collect::<Vec<_>>().join(" ")))
            })
            .unwrap_or_default();
        let is_new = row
            .select(&SEL_CABINET_NEW)
            .next()
            .map(|el| !el.value().classes().any(|class| class == "not-new"))
            .unwrap_or(false);
        let list_id = row.value().attr("id").unwrap_or_default().to_string();
        let url = cabinet_direct_url(&list_url, &cabinet_id);

        items.push(KwicCabinetItem {
            cabinet_id,
            list_id,
            name,
            level,
            updated_at,
            is_new,
            url,
        });
    }

    KwicCabinetReference {
        title: "学生キャビネット".to_string(),
        items,
        raw_html_debug: None,
    }
}
