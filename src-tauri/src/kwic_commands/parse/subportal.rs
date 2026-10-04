use crate::config;

use super::super::types::{KwicPortalNotification, KwicSubportalData, KwicSubportalLink};
use super::{
    SEL_SUBPORTAL_CAT, SEL_SUBPORTAL_DATE, SEL_SUBPORTAL_DEPT, SEL_SUBPORTAL_LI,
    SEL_SUBPORTAL_LINK, SEL_SUBPORTAL_TITLE, SEL_SUBPORTAL_TITLE_SPAN, SEL_SYSTEM_IMAGE,
};

/// Parse a KWIC Portal subportal page.
/// Subportal pages contain link lists and notification items similar to the home page.
pub(in crate::kwic_commands) fn parse_subportal(html: &str) -> KwicSubportalData {
    use scraper::Html;
    let doc = Html::parse_document(html);

    // Page title: .subportal-title-txt
    let page_title = doc
        .select(&SEL_SUBPORTAL_TITLE)
        .next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    // Links: li.subportal-block-relation-list-li a.subportal-block-txtlink-li-b
    // Each <a> contains <img class="systemlink-image"> (icon) + <span> (title)
    let mut links = Vec::new();
    for a in doc.select(&SEL_SUBPORTAL_LINK) {
        let title: String = a.text().collect::<Vec<_>>().join("").trim().to_string();
        let href = a.value().attr("href").unwrap_or_default();
        if title.is_empty() || href.is_empty() || href == "#" {
            continue;
        }
        if href.starts_with("javascript:") {
            continue;
        }
        let url = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("{}{}", config::KWIC_BASE, href)
        };
        let icon_url = a
            .select(&SEL_SYSTEM_IMAGE)
            .next()
            .and_then(|img| img.value().attr("src"))
            .map(|src| {
                if src.starts_with("http") {
                    src.to_string()
                } else {
                    format!("{}{}", config::KWIC_BASE, src)
                }
            })
            .unwrap_or_default();
        if links.iter().any(|l: &KwicSubportalLink| l.url == url) {
            continue;
        }
        links.push(KwicSubportalLink {
            title,
            url,
            icon_url,
            description: String::new(),
        });
    }

    // Notifications: li.subportal-block-info-list-li
    // Structure per item:
    //   .subportal-block-list-li-txt-info1 = category
    //   .subportal-block-list-li-txt-info2 span.link-txt[data1][data2] = title + id + type
    //   .subportal-block-list-li-txt-info3 span:first = date
    //   .subportal-block-list-li-txt-info4 = department
    let mut notifications = Vec::new();
    for li in doc.select(&SEL_SUBPORTAL_LI) {
        let title_el = match li.select(&SEL_SUBPORTAL_TITLE_SPAN).next() {
            Some(el) => el,
            None => continue,
        };

        let id = title_el
            .value()
            .attr("data1")
            .unwrap_or_default()
            .to_string();
        let data2 = title_el
            .value()
            .attr("data2")
            .unwrap_or_default()
            .to_string();
        let title = title_el
            .text()
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }

        let category = li
            .select(&SEL_SUBPORTAL_CAT)
            .next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();

        let date = li
            .select(&SEL_SUBPORTAL_DATE)
            .next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();

        let dept = li
            .select(&SEL_SUBPORTAL_DEPT)
            .next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();

        notifications.push(KwicPortalNotification {
            id,
            title,
            date,
            category: if !dept.is_empty() { dept } else { category },
            important: false,
            information_type: data2,
            // Subportal notifications only have data1/data2 in onclick
            person_category_cd: String::new(),
            category_cd: String::new(),
        });
    }

    KwicSubportalData {
        title: page_title,
        links,
        notifications,
    }
}
