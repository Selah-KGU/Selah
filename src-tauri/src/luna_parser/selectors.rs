use scraper::Selector;
use std::sync::LazyLock;

macro_rules! sel {
    ($name:ident, $s:expr) => {
        pub(super) static $name: LazyLock<Selector> =
            LazyLock::new(|| Selector::parse($s).unwrap());
    };
}

// ── Timetable selectors ──
sel!(SEL_DATA_ROW, ".div-table-data-row");
sel!(SEL_PERIOD_COL, ".div-table-colomn-period");
sel!(SEL_TABLE_CELL, ".div-table-cell");
sel!(SEL_COURSE_BTN, ".timetable-course-top-btn");
sel!(SEL_CELL_DETAIL, ".div-table-cell-detail span");
sel!(
    SEL_COMMUNITY_BTN,
    ".timetable-community-course .timetable-course-top-btn"
);

// ── Todo selectors ──
sel!(SEL_TODO_LIST, ".todo-list");
sel!(SEL_TODO_COURSE, ".todolist-course");
sel!(SEL_TODO_TYPE, ".todolist-contents-type span");
sel!(SEL_TODO_NAME, ".todolist-contents-name a");
sel!(SEL_TODO_DEADLINE, ".todolist-mobile-width-deadline");
sel!(SEL_TODO_STATUS, ".todolist-contents-status span");
sel!(
    SEL_TODO_FEEDBACK,
    ".todolist-feedback .todolist-mobile-feedback"
);

// ── Notification selectors ──
sel!(SEL_NOTIF_LIST, ".update-info-list");
sel!(SEL_NOTIF_DATE, ".update-info-updateDate label");
sel!(SEL_NOTIF_COURSE, ".update-info-courseInfo span");
sel!(SEL_NOTIF_MODULE, ".update-info-module span");
sel!(SEL_NOTIF_CONTENT, ".update-info-contents .break-word");
sel!(SEL_NOTIF_URL, ".updateInfoUrl");
sel!(SEL_INPUT_IDNUMBER, "input[id='idnumber']");
sel!(SEL_INPUT_IDNAME, "input[name='idnumber']");

// ── Common shared selectors ──
sel!(SEL_DETAIL_VERT, ".contents-detail.contents-vertical");
sel!(
    SEL_BLOCK_DETAIL,
    ".block > .contents-list > .contents-detail.contents-vertical"
);
sel!(SEL_HEADER_BOLD, ".contents-header-txt .bold-txt");
sel!(
    SEL_HEADER_COMBO,
    ".contents-header-txt .bold-txt, .contents-header-txt"
);
sel!(SEL_INPUT_AREA, ".contents-input-area");
sel!(SEL_DOWNLOAD_FILE, ".downloadFile");
sel!(SEL_OBJECT_NAME, ".objectName");
sel!(SEL_HIDDEN_INPUT, "input[type='hidden']");

// ── Discussion selectors ──
sel!(
    SEL_THEME_TOP,
    "#themeTopList .result-list, #themeTopList .contents-result-list"
);
sel!(SEL_THREAD_TITLE, ".theme-top-thread-title.link-txt");
sel!(SEL_THREAD_AUTHOR, ".theme-top-thread-author");
sel!(SEL_THREAD_DATE, ".theme-top-thread-createdate");
sel!(SEL_THREAD_STATUS, ".theme-top-thread-postzyoukyou");

// ── Thread post selectors ──
sel!(SEL_THREAD_POST_BLOCK, "#threadPostListArea .clearfix");
sel!(SEL_POST_CONTENTS_TEXT, ".postContentsText");
sel!(SEL_POST_DATE, ".postDate");
sel!(SEL_POST_USER, ".postUser");
sel!(SEL_POST_ID, ".postId");
sel!(SEL_MSG_BLOCK, ".discussion-message-block");
sel!(SEL_DISCUSS_MESS_FILE, ".discuss_mess_file");

// ── Inquiry (お問い合わせ / メッセージ) selectors ──
sel!(SEL_INQUIRY_FORM, "#inquirySetForm");
sel!(SEL_INQUIRY_MSG_BLOCK, ".discussion-message-block");
sel!(SEL_INQUIRY_MSG_MAIN, ".discussion-message-main");
sel!(SEL_INQUIRY_MSG_FILE, ".discuss_mess_file");
sel!(SEL_INQUIRY_QL_EDITOR, ".ql-editor");
sel!(SEL_INQUIRY_MSG_FOOTER, ".message-margin-top");
sel!(SEL_INQUIRY_HIDDEN_POSTID, ".contents-hidden.postId");
sel!(SEL_INQUIRY_HIDDEN_CONTENTS, ".contents-hidden.contents");
sel!(SEL_INQUIRY_BLOCK_TITLE, ".block-title .block-title-txt");
sel!(SEL_INQUIRY_POSTFILE_FORM, "#inquiryPostFile");
sel!(SEL_INQUIRY_UPFILE_FORM, "#inquiryFileForm");
sel!(SEL_INQUIRY_FILENAME_INPUT, "input.fileName");
sel!(SEL_INQUIRY_OBJECTNAME_INPUT, "input.objectName");
sel!(SEL_INQUIRY_POSTID_INPUT, "input.postId");
sel!(SEL_INQUIRY_SCANSTATUS_INPUT, "input.scanStatus");

// ── Detail page selectors ──
sel!(SEL_REPORT_FORM, "#reportDownloadForm");
sel!(SEL_FORUMS_FORM, "#forumsPostFile");
// Luna announcement attachment hidden inputs
sel!(SEL_CMT_FILENAME, "input.cmtInfoFileName");
sel!(SEL_CMT_OBJECTNAME, "input.cmtInfoObjectName");
sel!(SEL_TEMPFILE_LINK, "a[href*='tempfile']");
sel!(SEL_DOWNLOAD_LINK, "a[href*='download']");
sel!(
    SEL_VIDEO_LINK,
    ".block-list-video a[href], .examination-movie a[href]"
);
sel!(
    SEL_BODY_LINK,
    ".contents-input-area a[href], .ql-editor a[href]"
);

// ── Forum post fallback selectors (detail page) ──
sel!(SEL_FORUM_POST_THREAD_AREA, ".thread-post-area");
sel!(SEL_FORUM_POST_LIST_BODY, ".post-list-area .post-body");
sel!(SEL_FORUM_POST_CONTENT, ".forum-post-content");
sel!(SEL_FORUMS_THREAD_CONTENT, ".forums-thread-content");

// ── Course top selectors ──
sel!(SEL_INFO_RESULT, ".course-result-list.sp-contents-hidden");
sel!(SEL_INFO_NAME_A, ".class-view-information-name a");
sel!(SEL_INFO_PRIORITY, ".portal-information-priority");
sel!(SEL_INFO_START, ".class-view-information-start");
sel!(SEL_INFO_END, ".class-view-information-end");
sel!(SEL_ONLINE_LINK, "#online .online-link a[href]");
sel!(SEL_READMORE_DIV, ".contents-detail-readmore-txt div");
sel!(SEL_READMORE_SPAN, ".contents-detail-readmore-txt span");
sel!(SEL_SYLLABUS_LINK, ".class-header-syllabus");
sel!(SEL_GRADE_LINK, "a[href*='external_grade']");
sel!(
    SEL_SIDE_MENU,
    "#sidemenuListMessage a[onclick], #sidemenuListEdit a[onclick]"
);
sel!(SEL_MATERIAL_LIST, "#courseContent #materialList");
sel!(SEL_MAT_TITLE, ".course-material-title-txt");
sel!(SEL_INPUT_SPAN, ".contents-input-area span");
sel!(SEL_MAT_FILE_NAME, ".material-file-name");
sel!(SEL_MAT_CSS, ".course-result-list.materialCss");
sel!(SEL_QL_EDITOR, ".ql-editor");
sel!(SEL_SCRIPT, "script");
sel!(SEL_FILENAME, ".fileName");
sel!(SEL_RESOURCE_ID, ".resource_Id");
sel!(SEL_FILETYPE, ".fileType");
sel!(SEL_DL_MAT_ID, "#dlMaterialId");
sel!(SEL_OPEN_END_DATE, ".openEndDate");
sel!(SEL_SCAN_STATUS, ".scanStatus");

// ── Report/Exam/Discussion list selectors ──
sel!(SEL_REPORT_LIST, "#report .contents-result-list");
sel!(SEL_RPT_NAME, ".course-view-report-name.link-txt");
sel!(SEL_RPT_START, ".course-view-report-time-start");
sel!(SEL_RPT_END, ".course-view-report-time-end");
sel!(SEL_RPT_STATUS, ".course-view-report-status");
sel!(SEL_EXAM_LIST, "#examination .contents-result-list");
sel!(SEL_EXAM_NAME, ".course-view-examination-name.link-txt");
sel!(SEL_EXAM_NAME_FB, ".course-view-examination-name");
sel!(SEL_LINK_TXT, "a.link-txt");
sel!(
    SEL_EXAM_PERIOD,
    ".course-view-examination-period.sp-contents-hidden"
);
sel!(SEL_EXAM_STATUS, ".course-view-examination-answer-status");
sel!(SEL_DISC_LIST, "#discussion .contents-result-list");
sel!(SEL_DISC_NAME, ".course-view-forum-title.link-txt");
sel!(SEL_DISC_NAME_FB, ".course-view-forum-title");
sel!(
    SEL_DISC_PERIOD,
    ".course-view-forum-period.sp-contents-hidden"
);
sel!(SEL_DISC_STATUS, ".course-view-forum-postzyoukyou");

// ── Survey/questionnaire list selectors ──
sel!(
    SEL_SURVEY_LIST,
    "#questionnaire .course-result-list, #courseViewSurveyList .course-result-list"
);
sel!(SEL_SURV_NAME, ".course-view-questionnaire-name.link-txt");
sel!(SEL_SURV_NAME_FB, ".course-view-questionnaire-name");
sel!(
    SEL_SURV_PERIOD,
    ".course-view-questionnaire-period.sp-contents-hidden"
);
sel!(SEL_SURV_STATUS, ".course-view-questionnaire-answer-status");

// ── Attendance selectors ──
sel!(
    SEL_ATT_LIST,
    "#attendance .course-result-list.contents-display-flex"
);
sel!(SEL_ATT_TITLE, ".course-view-attendance-title");
sel!(SEL_ATT_DATE, ".course-view-attendance-date");
sel!(SEL_ATT_STATUS, ".course-view-attendance-status");
sel!(SEL_ATT_ACTION_A, ".course-view-attendance-status a");

// ── Utility selectors ──
sel!(SEL_A_HREF, "a[href]");
sel!(SEL_SPAN, "span");
sel!(SEL_OPT_SELECTED, "option[selected]");
sel!(SEL_OPTION, "option");
