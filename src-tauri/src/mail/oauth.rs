use super::auth::{MS_REDIRECT_URI, MS_SCOPES};

pub(crate) struct LoginAttempt {
    pub lifetime: crate::oauth_lifecycle::Attempt,
    pub http: crate::oauth_http::Http,
    pub client_id: String,
    pub verifier: String,
    pub state: String,
    pub url: url::Url,
}

impl LoginAttempt {
    pub fn new(
        lifetime: crate::oauth_lifecycle::Attempt,
        http: crate::oauth_http::Http,
        client_id: String,
    ) -> Self {
        let (verifier, challenge) = crate::oauth_lifecycle::generate_pkce();
        let state = uuid::Uuid::new_v4().to_string();
        let mut url = url::Url::parse(&format!("{}/authorize", crate::config::MS_AUTHORITY))
            .expect("valid Microsoft authority");
        url.query_pairs_mut().extend_pairs([
            ("client_id", client_id.as_str()),
            ("response_type", "code"),
            ("redirect_uri", MS_REDIRECT_URI),
            ("scope", MS_SCOPES),
            ("response_mode", "query"),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ]);
        Self {
            lifetime,
            http,
            client_id,
            verifier,
            state,
            url,
        }
    }
}

/// Unrelated or malformed navigations cannot complete an authorization attempt.
/// Validate errors as well as codes, and reject ambiguous duplicate parameters.
pub(crate) fn callback(url: &url::Url, expected_state: &str) -> Option<Result<String, String>> {
    let redirect = url::Url::parse(MS_REDIRECT_URI).expect("valid redirect");
    if url.scheme() != redirect.scheme()
        || url.host_str() != redirect.host_str()
        || url.port_or_known_default() != redirect.port_or_known_default()
        || url.path() != redirect.path()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    let one = |name: &str| {
        let mut values = pairs
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_ref());
        match (values.next(), values.next()) {
            (Some(value), None) if !value.is_empty() => Some(value),
            _ => None,
        }
    };
    if one("state") != Some(expected_state) || expected_state.is_empty() {
        return None;
    }
    let has_code = pairs.iter().any(|(key, _)| key == "code");
    let has_error = pairs.iter().any(|(key, _)| key == "error");
    match (has_code, has_error) {
        (true, false) => one("code").map(|code| Ok(code.to_owned())),
        (false, true) => {
            one("error").map(|error| Err(format!("Microsoft authorization failed: {error}")))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_one_code_or_error_bound_to_exact_redirect_and_state() {
        let parse = |url: &str| callback(&url::Url::parse(url).unwrap(), "expected");
        assert_eq!(
            parse("http://localhost/?state=expected&code=code"),
            Some(Ok("code".into()))
        );
        assert!(
            parse("http://localhost/?state=expected&error=access_denied")
                .unwrap()
                .is_err()
        );
        for url in [
            "http://localhost/?code=code",
            "http://localhost/?state=wrong&code=code",
            "https://localhost/?state=expected&code=code",
            "http://localhost:1234/?state=expected&code=code",
            "http://localhost/other?state=expected&code=code",
            "http://localhost.evil.test/?state=expected&code=code",
            "http://user@localhost/?state=expected&code=code",
            "http://localhost/?state=expected&code=code#fragment",
            "http://localhost/?state=expected&state=wrong&code=code",
            "http://localhost/?state=expected&code=one&code=two",
            "http://localhost/?state=expected&code=code&error=access_denied",
            "http://localhost/?state=expected&code=",
            "http://localhost/?state=wrong&error=access_denied",
        ] {
            assert!(parse(url).is_none(), "accepted {url}");
        }
    }

    #[test]
    fn each_attempt_has_independent_state_and_s256_verifier() {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let mut lifecycle = crate::oauth_lifecycle::Lifecycle::default();
        let first = LoginAttempt::new(
            lifecycle.begin(),
            crate::oauth_http::Http::new(),
            "test & client".into(),
        );
        let second = LoginAttempt::new(
            lifecycle.begin(),
            crate::oauth_http::Http::new(),
            "test".into(),
        );
        assert_ne!(first.state, second.state);
        assert_ne!(first.verifier, second.verifier);
        let params: std::collections::HashMap<_, _> =
            first.url.query_pairs().into_owned().collect();
        assert_eq!(params["client_id"], "test & client");
        assert_eq!(params["state"], first.state);
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(
            params["code_challenge"],
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(first.verifier.as_bytes()))
        );
        assert!(!first.url.as_str().contains(&first.verifier));
    }
}
