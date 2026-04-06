/// Base URLs for all backend services
pub const KG_COURSE_BASE: &str = "https://kg-course.kwansei.ac.jp";
pub const LUNA_BASE: &str = "https://luna.kwansei.ac.jp";
pub const KWIC_BASE: &str = "https://kwic.kwansei.ac.jp";

/// SAML login entry points
pub const LUNA_SAML_URL: &str = "https://luna.kwansei.ac.jp/saml/login?disco=true";
pub const KWIC_SAML_URL: &str = "https://kwic.kwansei.ac.jp/saml/login?disco=true";

/// Microsoft Graph / OAuth
pub const MS_AUTHORITY: &str = "https://login.microsoftonline.com/common/oauth2/v2.0";
pub const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

/// Period time slots (start_hour, start_min, end_hour, end_min)
pub const PERIOD_TIMES: [(u32, u32, u32, u32); 7] = [
    (9, 0, 10, 30),   // 1限
    (11, 0, 12, 30),  // 2限
    (13, 30, 15, 0),  // 3限
    (15, 10, 16, 40), // 4限
    (16, 50, 18, 20), // 5限
    (18, 30, 20, 0),  // 6限
    (20, 10, 21, 40), // 7限
];
