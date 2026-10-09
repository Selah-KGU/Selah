//! KWIC portal HTML parsers.

use std::sync::LazyLock;

// ============ Cached Selectors ============

macro_rules! sel {
    ($name:ident, $s:expr) => {
        static $name: LazyLock<scraper::Selector> =
            LazyLock::new(|| scraper::Selector::parse($s).expect(concat!("bad selector: ", $s)));
    };
}

sel!(SEL_NOTICE_A, ".portal-notice-li a.portal-notice-li-a");
sel!(SEL_MAINLINK_A, ".portal-mainlink-li a");
sel!(SEL_INFO_A, "a.portal-info-content-li-a, a[data1]");
sel!(SEL_INFO_LI, "li.portal-info-content-li");
sel!(SEL_INFO_LIST_ROW, "#information_list .result-list");
sel!(
    SEL_INFO_LIST_TITLE,
    ".portal-information-list-title.sp-contents-hidden span.link-txt[data1], span.link-txt[data1]"
);
sel!(
    SEL_INFO_LIST_DATE,
    ".portal-information-list-date.sp-contents-hidden span, .portal-information-list-date span"
);
sel!(
    SEL_INFO_LIST_DIVISION,
    ".portal-information-list-division.sp-contents-hidden, .portal-information-list-division"
);
sel!(
    SEL_INFO_TYPE_SELECTED,
    r#"select#informationType option[selected]"#
);
sel!(SEL_INFO_DATE, ".portal-subblock-infolist-left-item2 > div");
sel!(
    SEL_INFO_TITLE,
    ".portal-subblock-infolist-left-item2 > span"
);
sel!(SEL_INFO_CATEGORY, ".portal-subblock-infolist-right");

sel!(SEL_CSRF, r#"input[name="_csrf"]"#);
sel!(SEL_BLOCK_TITLE, ".block-title-txt");
sel!(SEL_CONTENTS_HTML, "#contentsHtml");
sel!(SEL_OUTGOING_DIV, ".portal-information-outgoing-division");
sel!(SEL_CONTENTS_DETAIL, ".contents-detail");
sel!(SEL_HEADER_BOLD, ".contents-header-txt .bold-txt");
sel!(SEL_INPUT_AREA, ".contents-input-area");
sel!(SEL_FILE_OBJECT, ".file-object");
sel!(SEL_FILE_NAME, ".downloadFile, .fileName");
sel!(SEL_OBJECT_NAME, ".objectName");
sel!(SEL_SUBPORTAL_TITLE, ".subportal-title-txt");
sel!(
    SEL_SUBPORTAL_LINK,
    "li.subportal-block-relation-list-li a.subportal-block-txtlink-li-b"
);
sel!(SEL_SYSTEM_IMAGE, "img.systemlink-image");
sel!(SEL_SUBPORTAL_LI, "li.subportal-block-info-list-li");
sel!(SEL_SUBPORTAL_CAT, ".subportal-block-list-li-txt-info1");
sel!(
    SEL_SUBPORTAL_TITLE_SPAN,
    ".subportal-block-list-li-txt-info2 span.link-txt"
);
sel!(
    SEL_SUBPORTAL_DATE,
    ".subportal-block-list-li-txt-info3 span"
);
sel!(SEL_SUBPORTAL_DEPT, ".subportal-block-list-li-txt-info4");
sel!(SEL_CABINET_ROW, ".cabinetList .result-list.result-data");
sel!(
    SEL_CABINET_TITLE,
    ".cabinet-view-list-name .cabinetDisplayLink, .cabinet-view-list-name a"
);
sel!(SEL_CABINET_NEW, ".cabinet-view-list-new .cabinet-area-new");
sel!(SEL_CABINET_DATE, ".cabinet-view-list-createdate span");

// Tab-specific notification selectors
sel!(SEL_TAB1_LI, "#portalinfocontent1 li.portal-info-content-li");
sel!(SEL_TAB2_LI, "#portalinfocontent2 li.portal-info-content-li");
sel!(SEL_TAB3_LI, "#portalinfocontent3 li.portal-info-content-li");
sel!(SEL_TAB4_LI, "#portalinfocontent4 li.portal-info-content-li");

#[path = "parse/cabinet.rs"]
mod cabinet;
#[path = "parse/detail.rs"]
mod detail;
#[path = "parse/home.rs"]
mod home;
#[path = "parse/information.rs"]
mod information;
#[path = "parse/subportal.rs"]
mod subportal;

pub(in crate::kwic_commands) use cabinet::parse_cabinet_reference;
pub(in crate::kwic_commands) use detail::{
    compact_inline_images, extract_csrf_token, parse_detail_html,
};
pub(in crate::kwic_commands) use home::parse_portal_home;
pub(in crate::kwic_commands) use information::merge_information_list_sections;
#[cfg(test)]
pub(in crate::kwic_commands) use information::parse_information_list_items;
pub(in crate::kwic_commands) use subportal::parse_subportal;
