use axum::{
    body::Body,
    extract::State,
    http::{Request, Response, StatusCode},
    routing::get,
    Json, Router,
};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use scafra_security::{layer, JwtConfig, SecurityConfig};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::net::TcpListener;
use tokio::sync::{Barrier, Notify, RwLock, Semaphore};
use tower::ServiceExt;

const KEY_ID: &str = "security-test-rsa-key";
const JWKS_N: &str = "tEaY79VFcn9pvrOHgFR6RVjud03RgrxyuUOlJXJEF7-VuOsRk8350rygE1dQ0eQo-O6J3JxR5DzZh9Bd6zCQ2_7Jp6BWo26b6x0ZtnLKGQkezc5rbshS9y2r7SnyCsrAS9G_IGQpy2pnRvQq0_BlSnIo3qH8MarecboZl-sJ4PLDDNioC3cD6HkF5rCvyh_-YZGMO8t8DfPJX0erTTnE8ss69-WQq6BBA6HzVx2HcYpaP_hg_WN30ozenvTYGpCCdAAtzpuefpZlQn-jI5ZIvnAsBG7627rTH91GhHkbrVAZPPkQq5-G1zl6qfoD7tVONGRLCucx8qZvKzkHesiG6w";
const RSA_PRIVATE_KEY: &[u8] = include_bytes!("fixtures/rsa-private.pem");

#[derive(Serialize)]
struct Claims {
    exp: usize,
    iss: String,
    aud: String,
    scope: String,
}

fn token(issuer: &str, kid: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_owned());
    encode(
        &header,
        &Claims {
            exp: 4_000_000_000,
            iss: issuer.to_owned(),
            aud: "scafra-test-api".to_owned(),
            scope: "profile.read".to_owned(),
        },
        &EncodingKey::from_rsa_pem(RSA_PRIVATE_KEY).expect("fixture key should load"),
    )
    .expect("test token should encode")
}

fn valid_jwks() -> Value {
    valid_jwks_for(KEY_ID)
}

fn valid_jwks_for(kid: &str) -> Value {
    json!({
        "keys": [{
            "kid": kid,
            "kty": "RSA",
            "n": JWKS_N,
            "e": "AQAB"
        }]
    })
}

async fn serve_jwks(document: Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local JWKS listener should bind");
    let address = listener
        .local_addr()
        .expect("listener address is available");
    let app = Router::new().route(
        "/keys",
        get(move || {
            let document = document.clone();
            async move { Json(document) }
        }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{address}/keys")
}

async fn serve_oidc(document: Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local OIDC listener should bind");
    let address = listener
        .local_addr()
        .expect("listener address is available");
    let issuer = format!("http://{address}");
    let jwks_uri = format!("{issuer}/keys");
    let metadata = json!({ "jwks_uri": jwks_uri });
    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let metadata = metadata.clone();
                async move { Json(metadata) }
            }),
        )
        .route(
            "/keys",
            get(move || {
                let document = document.clone();
                async move { Json(document) }
            }),
        );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    issuer
}

#[derive(Clone)]
struct ControlledProvider {
    reply: Arc<RwLock<ProviderReply>>,
    requests: Arc<AtomicUsize>,
    gate: Arc<RwLock<Option<ProviderGate>>>,
}

#[derive(Clone)]
struct ProviderGate {
    started: Arc<Notify>,
    release: Arc<Semaphore>,
}

#[derive(Clone)]
struct ProviderReply {
    status: StatusCode,
    body: String,
}

impl ControlledProvider {
    fn new(document: Value) -> Self {
        Self {
            reply: Arc::new(RwLock::new(ProviderReply {
                status: StatusCode::OK,
                body: document.to_string(),
            })),
            requests: Arc::new(AtomicUsize::new(0)),
            gate: Arc::new(RwLock::new(None)),
        }
    }

    async fn replace(&self, status: StatusCode, body: impl Into<String>) {
        *self.reply.write().await = ProviderReply {
            status,
            body: body.into(),
        };
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    async fn block_responses(&self) -> ProviderGate {
        let gate = ProviderGate {
            started: Arc::new(Notify::new()),
            release: Arc::new(Semaphore::new(0)),
        };
        *self.gate.write().await = Some(gate.clone());
        gate
    }
}

async fn controlled_response(State(provider): State<ControlledProvider>) -> Response<Body> {
    provider.requests.fetch_add(1, Ordering::SeqCst);
    if let Some(gate) = provider.gate.read().await.clone() {
        gate.started.notify_one();
        let _ = gate.release.acquire().await;
    }
    let reply = provider.reply.read().await.clone();
    Response::builder()
        .status(reply.status)
        .header("content-type", "application/json")
        .body(Body::from(reply.body))
        .unwrap()
}

async fn serve_controlled(document: Value) -> (String, ControlledProvider) {
    let provider = ControlledProvider::new(document);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/keys", get(controlled_response))
        .with_state(provider.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{address}/keys"), provider)
}

fn protected_router(issuer: &str, jwk_set_uri: Option<String>) -> Router {
    layer(
        Router::new().route("/resource", get(|| async { "authorized" })),
        SecurityConfig {
            enabled: true,
            jwt: JwtConfig {
                enabled: true,
                issuer_uri: Some(issuer.to_owned()),
                jwk_set_uri,
                audiences: vec!["scafra-test-api".to_owned()],
                required_scopes: vec!["profile.read".to_owned()],
                ..JwtConfig::default()
            },
            ..SecurityConfig::default()
        },
    )
}

async fn authorize(router: Router, jwt: &str) -> (StatusCode, String) {
    let response = router
        .oneshot(
            Request::builder()
                .uri("/resource")
                .header("authorization", format!("Bearer {jwt}"))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should respond");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .expect("response body should be readable");
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn jwks_retrieval_validates_rsa_signature_and_claims() {
    let jwks_uri = serve_jwks(valid_jwks()).await;
    let issuer = "https://issuer.test";
    let router = protected_router(issuer, Some(jwks_uri));
    let valid = token(issuer, KEY_ID);
    let (status, body) = authorize(router, &valid).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body == "authorized");

    let valid = token(issuer, KEY_ID);
    let mut parts = valid.split('.');
    let header = parts.next().expect("token header should exist");
    let payload = parts.next().expect("token payload should exist");
    let signature = parts.next().expect("token signature should exist");
    let replacement = if signature.starts_with('A') { 'B' } else { 'A' };
    let invalid_signature = format!("{header}.{payload}.{replacement}{}", &signature[1..]);
    let jwks_uri = serve_jwks(valid_jwks()).await;
    let router = protected_router(issuer, Some(jwks_uri));
    let (status, body) = authorize(router, &invalid_signature).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("private key"));
}

#[tokio::test]
async fn oidc_discovery_fetches_keys_from_the_issuer_metadata() {
    let issuer = serve_oidc(valid_jwks()).await;
    let router = protected_router(&issuer, None);
    let valid = token(&issuer, KEY_ID);
    let (status, body) = authorize(router, &valid).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body == "authorized");
}

#[tokio::test]
async fn unavailable_or_invalid_jwks_fail_closed_without_leaking_credentials() {
    let issuer = "https://issuer.test";

    let unavailable = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local test address should bind");
    let unavailable_uri = format!(
        "http://{}/keys",
        unavailable
            .local_addr()
            .expect("listener address is available")
    );
    drop(unavailable);
    let router = protected_router(issuer, Some(unavailable_uri));
    let valid = token(issuer, KEY_ID);
    let (status, body) = authorize(router, &valid).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("private key"));

    for invalid_jwks in [
        json!({ "keys": [] }),
        json!({
            "keys": [{
                "kid": KEY_ID,
                "kty": "oct",
                "n": JWKS_N,
                "e": "AQAB"
            }]
        }),
        json!({ "keys": "malformed" }),
    ] {
        let jwks_uri = serve_jwks(invalid_jwks).await;
        let router = protected_router(issuer, Some(jwks_uri));
        let (status, body) = authorize(router, &valid).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!body.contains("private key"));
    }
}

#[tokio::test]
async fn unknown_kid_refreshes_once_and_accepts_a_rotated_key() {
    let (jwks_uri, provider) = serve_controlled(valid_jwks()).await;
    let issuer = "https://issuer.test";
    let router = protected_router(issuer, Some(jwks_uri));
    let old = token(issuer, KEY_ID);

    assert_eq!(authorize(router.clone(), &old).await.0, StatusCode::OK);
    assert_eq!(provider.request_count(), 1);

    provider
        .replace(StatusCode::OK, valid_jwks_for("rotated-key").to_string())
        .await;
    let rotated = token(issuer, "rotated-key");
    let (status, body) = authorize(router.clone(), &rotated).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "authorized");
    assert_eq!(provider.request_count(), 2);

    // The complete replacement removes the old key from this cache generation.
    let (status, body) = authorize(router, &old).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains(&old));
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn unknown_kid_is_denied_after_refresh_failure_and_is_rate_limited() {
    let (jwks_uri, provider) = serve_controlled(valid_jwks()).await;
    let router = protected_router("https://issuer.test", Some(jwks_uri));
    let known = token("https://issuer.test", KEY_ID);
    assert_eq!(authorize(router.clone(), &known).await.0, StatusCode::OK);
    assert_eq!(provider.request_count(), 1);

    provider
        .replace(
            StatusCode::BAD_GATEWAY,
            "provider-response-sentinel".to_owned(),
        )
        .await;
    for _ in 0..8 {
        let unknown = token("https://issuer.test", "attacker-kid");
        let (status, body) = authorize(router.clone(), &unknown).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!body.contains("provider-response-sentinel"));
        assert!(!body.contains(&unknown));
    }
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn malformed_and_non_success_refreshes_do_not_authorize_unknown_keys() {
    for (status, body) in [
        (StatusCode::OK, "{ malformed provider-response-sentinel"),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "provider-response-sentinel",
        ),
        (
            StatusCode::OK,
            r#"{"keys":[{"kid":"new-key","kty":"RSA","n":"bad","e":"AQAB"}]}"#,
        ),
    ] {
        let (jwks_uri, provider) = serve_controlled(valid_jwks()).await;
        let router = protected_router("https://issuer.test", Some(jwks_uri));
        let known = token("https://issuer.test", KEY_ID);
        assert_eq!(authorize(router.clone(), &known).await.0, StatusCode::OK);
        provider.replace(status, body).await;

        let unknown = token("https://issuer.test", "new-key");
        let (status, response_body) = authorize(router, &unknown).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!response_body.contains("provider-response-sentinel"));
        assert!(!response_body.contains("malformed"));
        assert!(!response_body.contains(&unknown));
    }
}

#[tokio::test]
async fn oversized_provider_response_is_rejected_without_leaking_its_body() {
    let (jwks_uri, provider) = serve_controlled(valid_jwks()).await;
    let router = protected_router("https://issuer.test", Some(jwks_uri));
    let known = token("https://issuer.test", KEY_ID);
    assert_eq!(authorize(router.clone(), &known).await.0, StatusCode::OK);

    let sentinel = "oversized-provider-response-sentinel";
    provider
        .replace(
            StatusCode::OK,
            format!("{sentinel}{}", "x".repeat(1024 * 1024)),
        )
        .await;
    let unknown = token("https://issuer.test", "new-key");
    let (status, body) = authorize(router, &unknown).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains(sentinel));
    assert!(!body.contains(&unknown));
}

#[tokio::test]
async fn remote_jwt_validation_rejects_bad_issuer_audience_expiration_and_algorithms() {
    let jwks_uri = serve_jwks(valid_jwks()).await;
    let issuer = "https://issuer.test";
    let router = protected_router(issuer, Some(jwks_uri));
    let invalid = [
        token_with_claims(
            KEY_ID,
            json!({"iss":"https://other.test","aud":"scafra-test-api","exp":4000000000_u64,"scope":"profile.read"}),
            Algorithm::RS256,
        ),
        token_with_claims(
            KEY_ID,
            json!({"iss":issuer,"aud":"other-api","exp":4000000000_u64,"scope":"profile.read"}),
            Algorithm::RS256,
        ),
        token_with_claims(
            KEY_ID,
            json!({"iss":issuer,"aud":"scafra-test-api","exp":1,"scope":"profile.read"}),
            Algorithm::RS256,
        ),
        token_with_claims(
            KEY_ID,
            json!({"iss":issuer,"aud":"scafra-test-api","exp":4000000000_u64,"nbf":4000000000_u64,"scope":"profile.read"}),
            Algorithm::RS256,
        ),
        token_with_claims(
            KEY_ID,
            json!({"iss":issuer,"aud":"scafra-test-api","exp":4000000000_u64,"scope":"profile.read"}),
            Algorithm::HS256,
        ),
    ];

    for token in invalid {
        let (status, body) = authorize(router.clone(), &token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!body.contains(&token));
    }
}

#[tokio::test]
async fn every_supported_rsa_algorithm_validates_with_the_cached_key() {
    let jwks_uri = serve_jwks(valid_jwks()).await;
    let issuer = "https://issuer.test";
    let router = protected_router(issuer, Some(jwks_uri));
    for algorithm in [Algorithm::RS256, Algorithm::RS384, Algorithm::RS512] {
        let jwt = token_with_claims(
            KEY_ID,
            json!({
                "iss": issuer,
                "aud": "scafra-test-api",
                "exp": 4_000_000_000_u64,
                "scope": "profile.read"
            }),
            algorithm,
        );
        let (status, body) = authorize(router.clone(), &jwt).await;
        assert_eq!(status, StatusCode::OK, "{algorithm:?}");
        assert_eq!(body, "authorized");
    }
}

fn token_with_claims(kid: &str, claims: Value, algorithm: Algorithm) -> String {
    let mut header = Header::new(algorithm);
    header.kid = Some(kid.to_owned());
    let key = if algorithm == Algorithm::HS256 {
        EncodingKey::from_secret(b"wrong-algorithm-secret")
    } else {
        EncodingKey::from_rsa_pem(RSA_PRIVATE_KEY).unwrap()
    };
    encode(&header, &claims, &key).unwrap()
}

#[tokio::test]
async fn jwks_algorithm_and_signing_metadata_must_match_the_token() {
    for key_metadata in [
        json!({"alg":"RS512"}),
        json!({"use":"enc"}),
        json!({"key_ops":["encrypt"]}),
    ] {
        let mut key = valid_jwks()["keys"][0].clone();
        for (name, value) in key_metadata.as_object().unwrap() {
            key[name] = value.clone();
        }
        let jwks_uri = serve_jwks(json!({"keys":[key]})).await;
        let router = protected_router("https://issuer.test", Some(jwks_uri));
        let token = token("https://issuer.test", KEY_ID);
        let (status, body) = authorize(router, &token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!body.contains(&token));
    }
}

#[tokio::test]
async fn concurrent_unknown_kid_requests_share_one_refresh() {
    let (jwks_uri, provider) = serve_controlled(valid_jwks()).await;
    let issuer = "https://issuer.test";
    let router = protected_router(issuer, Some(jwks_uri));
    let original = token(issuer, KEY_ID);
    assert_eq!(authorize(router.clone(), &original).await.0, StatusCode::OK);
    assert_eq!(provider.request_count(), 1);

    provider
        .replace(StatusCode::OK, valid_jwks_for("rotated-key").to_string())
        .await;
    let gate = provider.block_responses().await;
    let requests = 12;
    let barrier = Arc::new(Barrier::new(requests + 1));
    let mut tasks = tokio::task::JoinSet::new();
    let rotated = token(issuer, "rotated-key");
    for _ in 0..requests {
        let router = router.clone();
        let barrier = barrier.clone();
        let rotated = rotated.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            authorize(router, &rotated).await.0
        });
    }
    barrier.wait().await;
    gate.started.notified().await;
    for _ in 0..requests {
        tokio::task::yield_now().await;
    }
    gate.release.add_permits(requests);

    while let Some(result) = tasks.join_next().await {
        assert_eq!(result.unwrap(), StatusCode::OK);
    }
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn independent_routers_keep_independent_jwks_state() {
    let (working_uri, working_provider) = serve_controlled(valid_jwks()).await;
    let (failing_uri, failing_provider) = serve_controlled(valid_jwks()).await;
    failing_provider
        .replace(StatusCode::SERVICE_UNAVAILABLE, "provider-failure-sentinel")
        .await;
    let issuer = "https://issuer.test";
    let working_router = protected_router(issuer, Some(working_uri));
    let failing_router = protected_router(issuer, Some(failing_uri));
    let jwt = token(issuer, KEY_ID);

    assert_eq!(authorize(working_router, &jwt).await.0, StatusCode::OK);
    assert_eq!(
        authorize(failing_router, &jwt).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(working_provider.request_count(), 1);
    assert_eq!(failing_provider.request_count(), 1);
}

#[tokio::test]
async fn discovery_failure_is_generic_and_fails_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let metadata = json!({"jwks_uri":"provider-response-sentinel"});
    let app = Router::new().route(
        "/.well-known/openid-configuration",
        get(move || {
            let metadata = metadata.clone();
            async move { Json(metadata) }
        }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let issuer = format!("http://{address}");
    let router = protected_router(&issuer, None);
    let jwt = token(&issuer, KEY_ID);
    let (status, body) = authorize(router, &jwt).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("provider-response-sentinel"));
    assert!(!body.contains(&jwt));
}

#[tokio::test]
async fn provider_failure_denies_before_the_protected_handler_runs() {
    let (jwks_uri, provider) = serve_controlled(valid_jwks()).await;
    provider
        .replace(StatusCode::SERVICE_UNAVAILABLE, "provider-body-sentinel")
        .await;
    let handler_calls = Arc::new(AtomicUsize::new(0));
    let handler_counter = handler_calls.clone();
    let router = layer(
        Router::new().route(
            "/resource",
            get(move || {
                let handler_counter = handler_counter.clone();
                async move {
                    handler_counter.fetch_add(1, Ordering::SeqCst);
                    "authorized"
                }
            }),
        ),
        SecurityConfig {
            enabled: true,
            jwt: JwtConfig {
                enabled: true,
                issuer_uri: Some("https://issuer.test".to_owned()),
                jwk_set_uri: Some(jwks_uri),
                audiences: vec!["scafra-test-api".to_owned()],
                ..JwtConfig::default()
            },
            ..SecurityConfig::default()
        },
    );

    let jwt = token("https://issuer.test", KEY_ID);
    let (status, body) = authorize(router, &jwt).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(handler_calls.load(Ordering::SeqCst), 0);
    assert!(!body.contains("provider-body-sentinel"));
    assert!(!body.contains(&jwt));
}
