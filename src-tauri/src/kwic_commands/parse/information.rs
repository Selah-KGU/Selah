use crate::config;

use super::super::types::{KwicPortalItem, KwicPortalSection};
use super::{
    SEL_INFO_A, SEL_INFO_CATEGORY, SEL_INFO_DATE, SEL_INFO_LI, SEL_INFO_LIST_DATE,
    SEL_INFO_LIST_DIVISION, SEL_INFO_LIST_ROW, SEL_INFO_LIST_TITLE, SEL_INFO_TITLE,
    SEL_INFO_TYPE_SELECTED, SEL_TAB1_LI, SEL_TAB2_LI, SEL_TAB3_LI, SEL_TAB4_LI,
};

fn notification_tab_selectors() -> [(&'static scraper::Selector, &'static str); 4] {
    [
        (&*SEL_TAB1_LI, "呼出し・重要なお知らせ"),
        (&*SEL_TAB2_LI, "学部・研究科からのお知らせ"),
        (&*SEL_TAB3_LI, "授業のお知らせ"),
        (&*SEL_TAB4_LI, "その他"),
    ]
}

fn section_allows_kwic_list_merge(title: &str) -> bool {
    matches!(
        title,
        "呼出し・重要なお知らせ" | "学部・研究科からのお知らせ" | "その他"
    )
}

fn information_type_section_title(information_type: &str) -> Option<&'static str> {
    match information_type {
        "10" => Some("呼出し・重要なお知らせ"),
        "12" => Some("その他"),
        _ => None,
    }
}

fn apply_info_item_data(
    mut item: KwicPortalItem,
    data2: String,
    data3: String,
    data4: String,
) -> KwicPortalItem {
    item.information_type = data2;
    item.person_category_cd = data3;
    item.category_cd = data4;
    item
}

fn notification_item_key(item: &KwicPortalItem) -> String {
    if !item.id.trim().is_empty() {
        return format!("id:{}", item.id.trim());
    }
    format!("text:{}|{}", item.title.trim(), item.date.trim())
}

fn push_unique_notification_item(section: &mut KwicPortalSection, item: KwicPortalItem) -> bool {
    let key = notification_item_key(&item);
    if section
        .items
        .iter()
        .any(|existing| notification_item_key(existing) == key)
    {
        return false;
    }
    section.items.push(item);
    true
}

fn merge_items_into_section(
    sections: &mut Vec<KwicPortalSection>,
    title: &str,
    items: Vec<KwicPortalItem>,
) -> usize {
    if items.is_empty() {
        return 0;
    }
    if let Some(section) = sections.iter_mut().find(|section| section.title == title) {
        let mut added = 0;
        for item in items {
            if push_unique_notification_item(section, item) {
                added += 1;
            }
        }
        added
    } else {
        let count = items.len();
        sections.push(KwicPortalSection {
            title: title.to_string(),
            items,
        });
        count
    }
}

fn selected_information_type(document: &scraper::Html, fallback: Option<&str>) -> String {
    document
        .select(&SEL_INFO_TYPE_SELECTED)
        .next()
        .and_then(|option| option.value().attr("value"))
        .filter(|value| !value.trim().is_empty())
        .or(fallback)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(in crate::kwic_commands) fn parse_information_list_items(
    document: &scraper::Html,
    fallback_information_type: Option<&str>,
) -> Option<(&'static str, Vec<KwicPortalItem>)> {
    let information_type = selected_information_type(document, fallback_information_type);
    let section_title = information_type_section_title(&information_type)?;
    let mut items = Vec::new();

    for row in document.select(&SEL_INFO_LIST_ROW) {
        let Some(title_el) = row.select(&SEL_INFO_LIST_TITLE).next() else {
            continue;
        };
        let id = title_el
            .value()
            .attr("data1")
            .unwrap_or_default()
            .trim()
            .to_string();
        let category_cd = title_el
            .value()
            .attr("data2")
            .unwrap_or_default()
            .trim()
            .to_string();
        let title = normalize_text(&title_el.text().collect::<Vec<_>>().join(" "));
        if id.is_empty() || title.is_empty() {
            continue;
        }

        let date = row
            .select(&SEL_INFO_LIST_DATE)
            .next()
            .map(|el| normalize_text(&el.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();
        let category = row
            .select(&SEL_INFO_LIST_DIVISION)
            .next()
            .map(|el| normalize_text(&el.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();

        items.push(KwicPortalItem {
            id: id.clone(),
            title,
            date,
            category,
            url: format!(
                "{}/portal/home/information/detail?informationId={}&directLink=1",
                config::KWIC_BASE,
                id
            ),
            important: false,
            information_type: information_type.clone(),
            person_category_cd: "0".to_string(),
            category_cd,
        });
    }

    Some((section_title, items))
}

pub(in crate::kwic_commands) fn merge_information_list_sections(
    sections: &mut Vec<KwicPortalSection>,
    html: &str,
    fallback_information_type: Option<&str>,
) -> (usize, usize) {
    use scraper::Html;
    let document = Html::parse_document(html);

    let mut merged_from_tabs = false;
    let mut parsed_count = 0;
    let mut merged_count = 0;
    for (selector, title) in notification_tab_selectors() {
        if !section_allows_kwic_list_merge(title) {
            continue;
        }
        let items: Vec<KwicPortalItem> = document
            .select(selector)
            .filter_map(|li| {
                parse_info_item(&li)
                    .map(|(item, d2, d3, d4)| apply_info_item_data(item, d2, d3, d4))
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        merged_from_tabs = true;
        parsed_count += items.len();
        if let Some(section) = sections.iter_mut().find(|section| section.title == title) {
            for item in items {
                if push_unique_notification_item(section, item) {
                    merged_count += 1;
                }
            }
        } else {
            merged_count += items.len();
            sections.push(KwicPortalSection {
                title: title.to_string(),
                items,
            });
        }
    }
    if merged_from_tabs {
        return (parsed_count, merged_count);
    }

    if let Some((section_title, items)) =
        parse_information_list_items(&document, fallback_information_type)
    {
        let parsed = items.len();
        let merged = merge_items_into_section(sections, section_title, items);
        if parsed > 0 {
            return (parsed, merged);
        }
    }

    let category_to_section = sections
        .iter()
        .filter(|section| section_allows_kwic_list_merge(&section.title))
        .flat_map(|section| {
            section
                .items
                .iter()
                .filter(|item| !item.category_cd.is_empty())
                .map(|item| (item.category_cd.clone(), section.title.clone()))
        })
        .collect::<std::collections::HashMap<_, _>>();

    for li in document.select(&SEL_INFO_LI) {
        let Some((item, d2, d3, d4)) = parse_info_item(&li) else {
            continue;
        };
        let Some(section_title) = category_to_section.get(&d4).cloned() else {
            continue;
        };
        let item = apply_info_item_data(item, d2, d3, d4);
        if let Some(section) = sections
            .iter_mut()
            .find(|section| section.title == section_title)
        {
            if push_unique_notification_item(section, item) {
                merged_count += 1;
            }
        }
    }
    (merged_count, merged_count)
}

/// Parse a single notification item from li.portal-info-content-li
/// Returns (KwicPortalItem, data2, data3, data4)
pub(in crate::kwic_commands::parse) fn parse_info_item(
    li: &scraper::ElementRef,
) -> Option<(KwicPortalItem, String, String, String)> {
    // Extract informationId and data attributes from `a[data1]`
    let a = li.select(&SEL_INFO_A).next()?;
    let id = a.value().attr("data1").unwrap_or_default().to_string();
    let data2 = a.value().attr("data2").unwrap_or_default().to_string();
    let data3 = a.value().attr("data3").unwrap_or_default().to_string();
    let data4 = a.value().attr("data4").unwrap_or_default().to_string();

    // Date: .portal-subblock-infolist-left-item2 > div
    let date = li
        .select(&SEL_INFO_DATE)
        .next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    // Title: .portal-subblock-infolist-left-item2 > span
    let mut title = li
        .select(&SEL_INFO_TITLE)
        .next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();
    if title.is_empty() {
        title = normalize_text(&a.text().collect::<Vec<_>>().join(" "));
    }

    if title.is_empty() {
        return None;
    }

    // Category/department: .portal-subblock-infolist-right
    let category = li
        .select(&SEL_INFO_CATEGORY)
        .next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    Some((
        KwicPortalItem {
            id: id.clone(),
            title,
            date,
            category,
            url: format!(
                "{}/portal/home/information/detail?informationId={}&directLink=1",
                config::KWIC_BASE,
                id
            ),
            important: false,
            information_type: String::new(),
            person_category_cd: String::new(),
            category_cd: String::new(),
        },
        data2,
        data3,
        data4,
    ))
}

pub(in crate::kwic_commands::parse) fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
