use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Json, Router,
};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use scafra_security::{layer, JwtConfig, SecurityConfig};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;
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
    json!({
        "keys": [{
            "kid": KEY_ID,
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
