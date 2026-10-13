use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use scafra_security::{layer, JwtConfig, SecurityConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

const JWT_SECRET: &str = "jwt-sentinel-secret";
const WRONG_SECRET: &str = "other-test-signing-secret";
const ISSUER: &str = "https://issuer.test";
const AUDIENCE: &str = "scafra-api";

fn token(claims: Value, secret: &str) -> String {
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("test JWT should encode")
}

fn router() -> Router {
    layer(
        Router::new().route("/resource", get(|| async { "authorized" })),
        SecurityConfig {
            enabled: true,
            jwt: JwtConfig {
                enabled: true,
                secret: Some(JWT_SECRET.to_owned()),
                issuer_uri: Some(ISSUER.to_owned()),
                audiences: vec![AUDIENCE.to_owned()],
                required_scopes: vec!["profile.read".to_owned()],
                ..JwtConfig::default()
            },
            ..SecurityConfig::default()
        },
    )
}

async fn send(router: &Router, credential: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::builder().uri("/resource");
    if let Some(credential) = credential {
        request = request.header("authorization", format!("Bearer {credential}"));
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::empty()).expect("request should build"))
        .await
        .expect("router should respond");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("response body should be readable");
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn claims(expiration: Option<usize>, issuer: &str, audience: &str, scope: &str) -> Value {
    let mut claims = json!({
        "iss": issuer,
        "aud": audience,
        "scope": scope
    });
    if let Some(expiration) = expiration {
        claims["exp"] = json!(expiration);
    }
    claims
}

#[tokio::test]
async fn hs256_validation_checks_signature_issuer_audience_and_expiration() {
    let router = router();
    let valid = token(
        claims(Some(4_000_000_000), ISSUER, AUDIENCE, "profile.read"),
        JWT_SECRET,
    );
    let (status, body) = send(&router, Some(&valid)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body == "authorized");

    let invalid = [
        Some(token(
            claims(Some(4_000_000_000), ISSUER, AUDIENCE, "profile.read"),
            WRONG_SECRET,
        )),
        Some(token(
            claims(
                Some(4_000_000_000),
                "https://wrong-issuer.test",
                AUDIENCE,
                "profile.read",
            ),
            JWT_SECRET,
        )),
        Some(token(
            claims(
                Some(4_000_000_000),
                ISSUER,
                "wrong-audience",
                "profile.read",
            ),
            JWT_SECRET,
        )),
        Some(token(
            claims(Some(1), ISSUER, AUDIENCE, "profile.read"),
            JWT_SECRET,
        )),
        Some(token(
            claims(None, ISSUER, AUDIENCE, "profile.read"),
            JWT_SECRET,
        )),
        Some("not-a-jwt".to_owned()),
        None,
    ];
    for credential in invalid {
        let (status, body) = send(&router, credential.as_deref()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!body.contains(JWT_SECRET));
        assert!(!body.contains(WRONG_SECRET));
        assert!(!body.contains("not-a-jwt"));
    }
}

#[tokio::test]
async fn valid_jwt_without_configured_scope_is_forbidden() {
    let router = router();
    let missing_scope = token(
        claims(Some(4_000_000_000), ISSUER, AUDIENCE, "profile.write"),
        JWT_SECRET,
    );
    let (status, body) = send(&router, Some(&missing_scope)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!body.contains(JWT_SECRET));
    assert!(!body.contains(&missing_scope));
}

#[tokio::test]
async fn direct_middleware_rejects_invalid_jwt_configuration() {
    let config = SecurityConfig {
        enabled: true,
        jwt: JwtConfig {
            enabled: true,
            secret: Some(String::new()),
            issuer_uri: Some(ISSUER.to_owned()),
            audiences: vec![AUDIENCE.to_owned()],
            ..JwtConfig::default()
        },
        ..SecurityConfig::default()
    };
    let router = layer(
        Router::new().route("/resource", get(|| async { "authorized" })),
        config,
    );
    let empty_secret_token = token(
        claims(Some(4_000_000_000), ISSUER, AUDIENCE, "profile.read"),
        "",
    );

    let (status, body) = send(&router, Some(&empty_secret_token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains(&empty_secret_token));
}

#[tokio::test]
async fn local_secret_mode_rejects_algorithms_other_than_hs256() {
    let mut header = Header::new(Algorithm::HS512);
    header.kid = Some("jwt-sentinel-kid".to_owned());
    let token = encode(
        &header,
        &claims(Some(4_000_000_000), ISSUER, AUDIENCE, "profile.read"),
        &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
    )
    .expect("test JWT should encode");

    let (status, body) = send(&router(), Some(&token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains(&token));
    assert!(!body.contains(JWT_SECRET));
}
