use super::CookieData;

pub(super) fn parse_response(json: &str) -> Result<serde_json::Value, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "Invalid cookie operation response".to_string())?;
    if !value.is_object() || value.get("error").is_some() {
        return Err("WebView2 rejected the cookie operation".into());
    }
    Ok(value)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CdpCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: f64,
    pub http_only: bool,
    pub secure: bool,
    pub session: bool,
    #[serde(default)]
    pub same_site: Option<String>,
    #[serde(default)]
    pub partition_key: Option<serde_json::Value>,
    #[serde(default)]
    pub partition_key_opaque: bool,
}

pub(super) fn set_cookie_params(c: &CookieData) -> serde_json::Value {
    let mut params = serde_json::json!({
        "name": c.name, "value": c.value, "path": c.path,
        "secure": c.secure, "httpOnly": c.http_only,
        "url": format!("https://{}/", c.domain.trim_start_matches('.')),
    });
    if !c.is_host_only() {
        params["domain"] = serde_json::json!(c.domain);
    }
    if let Some(expires) = c.expires_unix {
        params["expires"] = serde_json::json!(expires);
    }
    if let Some(site) = c.normalized_same_site() {
        params["sameSite"] = serde_json::json!(site);
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_failure_cannot_be_counted_as_a_successful_delete() {
        assert!(parse_response(r#"{"error":{"code":-1,"message":"denied"}}"#).is_err());
        assert!(parse_response("invalid JSON").is_err());
        assert!(parse_response("null").is_err());
        assert!(parse_response("{}").is_ok());
    }

    #[test]
    fn restore_preserves_host_scope_samesite_and_expiry() {
        let mut c = CookieData {
            name: "sid".into(),
            value: "fake".into(),
            domain: "sso.kwansei.ac.jp".into(),
            path: "/auth".into(),
            secure: true,
            http_only: true,
            expires_unix: None,
            same_site: Some("none".into()),
            host_only: Some(true),
        };
        let host = set_cookie_params(&c);
        assert!(host.get("domain").is_none());
        assert!(host.get("expires").is_none());
        assert_eq!(host["url"], "https://sso.kwansei.ac.jp/");
        assert_eq!(host["path"], "/auth");
        assert_eq!(host["sameSite"], "None");
        assert_eq!(host["httpOnly"], true);
        c.host_only = Some(false);
        c.domain = ".kwansei.ac.jp".into();
        c.expires_unix = Some(2_000_000_000.0);
        let domain = set_cookie_params(&c);
        assert_eq!(domain["domain"], ".kwansei.ac.jp");
        assert_eq!(domain["expires"], 2_000_000_000.0);
    }

    #[test]
    fn read_keeps_partition_identity_for_deletion_without_flattening_it() {
        let c: CdpCookie = serde_json::from_value(serde_json::json!({
            "name": "sid", "value": "fake", "domain": "sso.kwansei.ac.jp", "path": "/",
            "expires": -1, "httpOnly": true, "secure": true, "session": true,
            "partitionKey": {"topLevelSite": "https://kwansei.ac.jp", "hasCrossSiteAncestor": true},
            "partitionKeyOpaque": true
        }))
        .unwrap();
        assert!(c.partition_key.is_some());
        assert!(c.partition_key_opaque);
    }
}
