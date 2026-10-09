pub(crate) const LUNA_COOKIES_KEY: &str = "luna_cookie_jar";

/// Check if Luna response body indicates session expired
pub(crate) fn is_luna_session_expired(body: &str) -> bool {
    // Redirected to login page
    if body.contains("linkCommonLogin") && body.contains("class=\"login-body\"") {
        return true;
    }
    // SAML redirect
    if body.contains("sso.kwansei.ac.jp") && body.contains("SAMLRequest") {
        return true;
    }
    false
}

pub const LUNA_SESSION_EXPIRED_MSG: &str = "Lunaセッションが期限切れです。再ログインしてください。";
pub const LUNA_AUTH_REQUIRED_MSG: &str = "Lunaにログインしてください";
