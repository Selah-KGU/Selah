use super::*;

#[test]
fn authorization_uses_registration_and_separate_identity_plan_scopes() {
    let url = authorize_url(
        "urn:uuid:host",
        None,
        "http://127.0.0.1:54321/auth/callback",
        "state",
        "nonce",
        "challenge",
    );
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "dynamic_agent_client");
    assert_eq!(query["agent_name_hint"], "Selah");
    assert_eq!(query["resource"], RESOURCE);
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["nonce"], "nonce");
    assert!(query["scope"].split_whitespace().any(|s| s == PLAN_SCOPE));
    let existing = Account {
        client_id: "oaiapp_existing".into(),
        id_token: "test-id-hint".into(),
        email: "account@example.com".into(),
        ..Default::default()
    };
    let url = authorize_url(
        "urn:uuid:host",
        Some(&existing),
        "http://127.0.0.1:1234/auth/callback",
        "next-state",
        "next-nonce",
        "next-challenge",
    );
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "oaiapp_existing");
    assert_eq!(query["id_token_hint"], "test-id-hint");
    assert!(!query.contains_key("agent_name_hint"));
    assert!(!query.contains_key("prompt"));
    let mut identity_only = existing.clone();
    identity_only
        .apply(tokens(Some("openid email"), None), true)
        .unwrap();
    let url = authorize_url(
        "urn:uuid:host",
        Some(&identity_only),
        "http://127.0.0.1:1234/auth/callback",
        "state",
        "nonce",
        "challenge",
    );
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["prompt"], "consent");
}

#[test]
fn callback_binds_state_path_issuer_and_returning_registration() {
    assert!(parse_callback("/favicon.ico", "state", None)
        .unwrap()
        .is_none());
    assert!(parse_callback(
        "/auth/callback?code=x&client_id=oaiapp_a&state=wrong",
        "state",
        None
    )
    .unwrap()
    .is_none());
    assert!(parse_callback(
        "/auth/callback?code=x&client_id=oaiapp_a&state=state&state=state",
        "state",
        None
    )
    .unwrap()
    .is_none());
    assert!(parse_callback(
        "/auth/callback?code=x&client_id=dynamic_agent_client&state=state",
        "state",
        None
    )
    .is_err());
    assert!(parse_callback("/auth/callback?code=x&state=state", "state", None).is_err());
    assert!(parse_callback(
        "/auth/callback?code=x&client_id=oaiapp_b&state=state",
        "state",
        Some("oaiapp_a")
    )
    .is_err());
    assert!(parse_callback(
        "/auth/callback?code=x&client_id=oaiapp_a&state=state&iss=https://other.example",
        "state",
        None
    )
    .is_err());
    assert!(parse_callback(
        "/auth/callback?error=access_denied&state=state",
        "state",
        None
    )
    .is_err());
    assert_eq!(
        parse_callback(
            "/auth/callback?code=x&client_id=oaiapp_a&state=state",
            "state",
            None
        )
        .unwrap(),
        Some(("x".into(), "oaiapp_a".into()))
    );
    assert_eq!(
        parse_callback(
            "/auth/callback?code=y&state=state",
            "state",
            Some("oaiapp_a")
        )
        .unwrap(),
        Some(("y".into(), "oaiapp_a".into()))
    );
}

fn tokens(scope: Option<&str>, refresh: Option<&str>) -> Tokens {
    Tokens {
        access_token: "test-access".into(),
        refresh_token: refresh.map(str::to_owned),
        id_token: None,
        token_type: "Bearer".into(),
        expires_in: 3600,
        scope: scope.map(str::to_owned),
    }
}

#[test]
fn identity_only_is_not_plan_access_and_refresh_preserves_or_replaces_grants() {
    let mut account = Account::default();
    account
        .apply(tokens(Some("openid email"), Some("refresh-old")), true)
        .unwrap();
    assert!(account.connected());
    assert!(!account.plan_enabled());
    account
        .apply(tokens(Some(PLAN_SCOPE), Some("refresh-rotated")), true)
        .unwrap();
    account.apply(tokens(None, None), false).unwrap();
    assert!(account.plan_enabled());
    assert_eq!(account.refresh_token, "refresh-rotated");
    account
        .apply(tokens(Some("openid email"), None), false)
        .unwrap();
    assert!(!account.plan_enabled());
    account.clear_tokens();
    assert!(!account.connected());
    assert!(account.id_token.is_empty());
}

#[test]
fn new_grant_does_not_inherit_previously_granted_plan_permission() {
    let mut account = Account::default();
    account.apply(tokens(Some(PLAN_SCOPE), None), true).unwrap();
    account.apply(tokens(None, None), true).unwrap();
    assert!(!account.plan_enabled());
    assert!(account.refresh_token.is_empty());
    let mut invalid = tokens(Some(PLAN_SCOPE), None);
    invalid.token_type = "unknown".into();
    assert!(account.apply(invalid, true).is_err());
}

#[test]
fn only_terminal_refresh_errors_retire_credentials() {
    for code in [
        "invalid_grant",
        "invalid_refresh_token",
        "token_expired",
        "refresh_token_expired",
        "refresh_token_invalidated",
        "refresh_token_reused",
    ] {
        assert!(unusable_refresh(
            reqwest::StatusCode::BAD_REQUEST,
            &serde_json::json!({"error":code})
        ));
        assert!(!unusable_refresh(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            &serde_json::json!({"error":code})
        ));
        assert!(!unusable_refresh(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            &serde_json::json!({"error":code})
        ));
    }
    assert!(!unusable_refresh(
        reqwest::StatusCode::BAD_REQUEST,
        &serde_json::json!({"error":"invalid_client"})
    ));
}

#[test]
fn id_token_requires_signature_issuer_audience_expiry_subject_and_nonce() {
    use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
    // Public, generated test-only key. Never used by the app or a real account.
    let signing = EncodingKey::from_rsa_pem(include_bytes!("fixtures/test-only-rsa.pem")).unwrap();
    let key = DecodingKey::from_rsa_pem(include_bytes!("fixtures/test-only-rsa.pub.pem")).unwrap();
    let now = chrono::Utc::now().timestamp();
    let claims = serde_json::json!({"iss":ISSUER, "aud":"oaiapp_test", "sub":"subject", "iat":now,
        "exp":now+3600, "nonce":"nonce", "email":"test@example.com"});
    let sign = |claims: &serde_json::Value| {
        encode(&Header::new(Algorithm::RS256), claims, &signing).unwrap()
    };
    let token = sign(&claims);
    assert_eq!(
        verify_identity(&token, "oaiapp_test", "nonce", &key, Algorithm::RS256)
            .unwrap()
            .sub,
        "subject"
    );
    assert!(verify_identity(&token, "other-client", "nonce", &key, Algorithm::RS256).is_err());
    assert!(verify_identity(&token, "oaiapp_test", "wrong-nonce", &key, Algorithm::RS256).is_err());
    for (field, value) in [
        ("iss", serde_json::json!("https://other.example")),
        ("exp", serde_json::json!(now - 60)),
        ("sub", serde_json::json!("")),
        ("iat", serde_json::Value::Null),
    ] {
        let mut invalid = claims.clone();
        invalid[field] = value;
        assert!(
            verify_identity(
                &sign(&invalid),
                "oaiapp_test",
                "nonce",
                &key,
                Algorithm::RS256
            )
            .is_err(),
            "accepted invalid {field}"
        );
    }
    let mut forged = token.into_bytes();
    let signature = forged.iter().rposition(|b| *b == b'.').unwrap() + 5;
    forged[signature] = if forged[signature] == b'A' {
        b'B'
    } else {
        b'A'
    };
    assert!(verify_identity(
        std::str::from_utf8(&forged).unwrap(),
        "oaiapp_test",
        "nonce",
        &key,
        Algorithm::RS256
    )
    .is_err());
    assert!(verify_identity("anything", "oaiapp_test", "nonce", &key, Algorithm::HS256).is_err());
}

#[tokio::test]
async fn loopback_ignores_probes_accepts_fragmented_callback_and_cancels() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let attempt = LOGIN.lock().unwrap().begin();
    let worker = tokio::spawn(async move { callback(listener, "state", None, &attempt).await });
    let mut probe = tokio::net::TcpStream::connect(address).await.unwrap();
    probe
        .write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .unwrap();
    let mut reply = String::new();
    probe.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 400"));
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            b"GET /auth/callback?code=test-code&state=state&client_id=oaiapp_test HTTP/1.1\r\n",
        )
        .await
        .unwrap();
    tokio::task::yield_now().await;
    stream.write_all(b"Host: 127.0.0.1\r\n\r\n").await.unwrap();
    reply.clear();
    stream.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"));
    assert!(!reply.contains("test-code"));
    assert_eq!(
        worker.await.unwrap().unwrap(),
        ("test-code".into(), "oaiapp_test".into())
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let attempt = LOGIN.lock().unwrap().begin();
    let worker = tokio::spawn(async move { callback(listener, "state", None, &attempt).await });
    chatgpt_cancel_login().await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}
