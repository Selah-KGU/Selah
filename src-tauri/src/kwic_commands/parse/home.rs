use crate::config;

use super::super::types::{KwicPortalItem, KwicPortalSection};
use super::information::parse_info_item;
use super::{SEL_MAINLINK_A, SEL_NOTICE_A, SEL_TAB1_LI, SEL_TAB2_LI, SEL_TAB3_LI, SEL_TAB4_LI};

// ============ Parsers ============
// Based on actual KWIC Portal HTML structure (kwic.kwansei.ac.jp)
//
// Home page layout:
//   - .portal-notice: pinned important links
//   - .portal-mainlink: 9 category cards (授業・履修・成績, キャンパスライフ, etc.)
//   - .portal-info-tab: 4 notification tabs
//     - #portalinfocontent1: 呼出し・重要なお知らせ
//     - #portalinfocontent2: 学部・研究科からのお知らせ
//     - #portalinfocontent3: 授業のお知らせ
//     - #portalinfocontent4: その他
//   - Each notification item: li.portal-info-content-li
//     - a[data1=informationId]
//     - .portal-subblock-infolist-left-item2 > div (date)
//     - .portal-subblock-infolist-left-item2 > span (title)
//     - .portal-subblock-infolist-right (department/category)
//     - .portal-information-new (NEW badge)

pub(in crate::kwic_commands) fn parse_portal_home(html: &str) -> Vec<KwicPortalSection> {
    use scraper::Html;

    let document = Html::parse_document(html);
    let mut sections = Vec::new();

    // 1. Parse pinned important links (注目コンテンツ)
    {
        let sel = &*SEL_NOTICE_A;
        let items: Vec<KwicPortalItem> = document
            .select(sel)
            .filter_map(|a| {
                let title: String = a.text().collect::<Vec<_>>().join(" ").trim().to_string();
                let href = a.value().attr("href").unwrap_or_default();
                if title.is_empty() {
                    return None;
                }
                Some(KwicPortalItem {
                    id: String::new(),
                    title,
                    date: String::new(),
                    category: "注目".to_string(),
                    url: href.to_string(),
                    important: true,
                    information_type: String::new(),
                    person_category_cd: String::new(),
                    category_cd: String::new(),
                })
            })
            .collect();
        if !items.is_empty() {
            sections.push(KwicPortalSection {
                title: "注目コンテンツ".to_string(),
                items,
            });
        }
    }

    // 2. Parse notification tabs
    let tabs: [(&scraper::Selector, &str); 4] = [
        (&*SEL_TAB1_LI, "呼出し・重要なお知らせ"),
        (&*SEL_TAB2_LI, "学部・研究科からのお知らせ"),
        (&*SEL_TAB3_LI, "授業のお知らせ"),
        (&*SEL_TAB4_LI, "その他"),
    ];

    for (sel, tab_title) in &tabs {
        let items: Vec<KwicPortalItem> = document
            .select(sel)
            .filter_map(|li| {
                parse_info_item(&li).map(|(mut item, d2, d3, d4)| {
                    item.information_type = d2;
                    item.person_category_cd = d3;
                    item.category_cd = d4;
                    item
                })
            })
            .collect();
        if !items.is_empty() {
            sections.push(KwicPortalSection {
                title: tab_title.to_string(),
                items,
            });
        }
    }

    // 3. Parse main link categories (メインリンク)
    {
        let sel = &*SEL_MAINLINK_A;
        let items: Vec<KwicPortalItem> = document
            .select(sel)
            .filter_map(|a| {
                let title: String = a.text().collect::<Vec<_>>().join(" ").trim().to_string();
                let href = a.value().attr("href").unwrap_or_default();
                if title.is_empty() {
                    return None;
                }
                Some(KwicPortalItem {
                    id: String::new(),
                    title,
                    date: String::new(),
                    category: "リンク".to_string(),
                    url: if href.starts_with("http") {
                        href.to_string()
                    } else {
                        format!("{}{}", config::KWIC_BASE, href)
                    },
                    important: false,
                    information_type: String::new(),
                    person_category_cd: String::new(),
                    category_cd: String::new(),
                })
            })
            .collect();
        if !items.is_empty() {
            sections.push(KwicPortalSection {
                title: "メインリンク".to_string(),
                items,
            });
        }
    }

    sections
}
