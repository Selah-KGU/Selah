pub(crate) const KWIC_COOKIES_KEY: &str = "kwic_cookie_jar";

/// Check if KWIC Portal response indicates session expired
pub(crate) fn is_kwic_session_expired(body: &str) -> bool {
    // Redirected to login page (KWIC shows its own login form)
    if body.contains("linkCommonLogin") && body.contains(r#"class="login-body""#) {
        return true;
    }
    // KWIC-specific login page
    if body.contains("type=\"password\"") && body.contains("kwic.kwansei.ac.jp") {
        return true;
    }
    // SAML redirect
    if body.contains("sso.kwansei.ac.jp") && body.contains("SAMLRequest") {
        return true;
    }
    false
}

pub const KWIC_SESSION_EXPIRED_MSG: &str = "KWICセッションが期限切れです。再ログインしてください。";
pub const KWIC_AUTH_REQUIRED_MSG: &str = "KWICポータルにログインしてください";
