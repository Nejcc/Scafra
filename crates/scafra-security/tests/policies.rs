use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    routing::get,
    Router,
};
use jsonwebtoken::{encode, EncodingKey, Header};
use scafra_security::{
    layer_with_route_policies, AuthorizationPolicy, JwtConfig, RouteAuthorizationMetadata,
    SecurityConfig,
};
use serde::Serialize;
use tower::ServiceExt;

const ADMIN_GROUP: &[&str] = &["admin"];
const ROLE_GROUPS: &[&[&str]] = &[ADMIN_GROUP];
const ROUTE_SCOPES: &[&str] = &["reports.read"];
const JWT_SECRET: &str = "policy-sentinel-secret";

#[derive(Serialize)]
struct Claims {
    exp: usize,
    iss: &'static str,
    aud: &'static str,
    roles: Vec<&'static str>,
    scope: &'static str,
}

fn token(roles: Vec<&'static str>, scope: &'static str) -> String {
    encode(
        &Header::default(),
        &Claims {
            exp: 4_000_000_000,
            iss: "https://policy.example",
            aud: "policy-api",
            roles,
            scope,
        },
        &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
    )
    .expect("test token should encode")
}

fn policies() -> Vec<RouteAuthorizationMetadata> {
    vec![
        RouteAuthorizationMetadata {
            controller: "report_controller",
            method: "GET",
            prefix: "/policy",
            path: "/private",
            controller_policy: AuthorizationPolicy {
                mode: scafra_security::AuthorizationMode::Protected,
                role_groups: ROLE_GROUPS,
                required_scopes: &[],
            },
            route_policy: AuthorizationPolicy {
                mode: scafra_security::AuthorizationMode::Inherit,
                role_groups: &[],
                required_scopes: ROUTE_SCOPES,
            },
        },
        RouteAuthorizationMetadata {
            controller: "report_controller",
            method: "GET",
            prefix: "/policy",
            path: "/open",
            controller_policy: AuthorizationPolicy::protected(),
            route_policy: AuthorizationPolicy::public(),
        },
        RouteAuthorizationMetadata {
            controller: "user_controller",
            method: "GET",
            prefix: "/specific/users",
            path: "/{id}",
            controller_policy: AuthorizationPolicy::protected(),
            route_policy: AuthorizationPolicy::inherit(),
        },
        RouteAuthorizationMetadata {
            controller: "user_controller",
            method: "GET",
            prefix: "/specific/users",
            path: "/me",
            controller_policy: AuthorizationPolicy::protected(),
            route_policy: AuthorizationPolicy::public(),
        },
    ]
}

fn router() -> Router {
    let config = SecurityConfig {
        enabled: true,
        permit_all: vec!["/permit".to_owned()],
        jwt: JwtConfig {
            enabled: true,
            secret: Some(JWT_SECRET.to_owned()),
            issuer_uri: Some("https://policy.example".to_owned()),
            audiences: vec!["policy-api".to_owned()],
            ..JwtConfig::default()
        },
        ..SecurityConfig::default()
    };
    let router = Router::new()
        .route("/policy/private", get(|| async { "private" }))
        .route("/policy/open", get(|| async { "open" }))
        .route("/specific/users/{id}", get(|| async { "user" }))
        .route("/specific/users/me", get(|| async { "current user" }))
        .route("/permit", get(|| async { "permit" }))
        .route("/permit/", get(|| async { "trailing" }))
        .route("/unannotated", get(|| async { "unannotated" }));
    layer_with_route_policies(router, config, policies())
}

async fn send(router: &Router, method: Method, path: &str, token: Option<&str>) -> StatusCode {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    router
        .clone()
        .oneshot(request.body(Body::empty()).expect("request should build"))
        .await
        .expect("router should respond")
        .status()
}

#[tokio::test]
async fn route_policies_allow_public_routes_and_deny_unmet_role_or_scope_requirements() {
    let router = router();
    assert_eq!(
        send(&router, Method::GET, "/policy/open", None).await,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, Method::GET, "/policy/private", None).await,
        StatusCode::UNAUTHORIZED
    );

    let missing_role = token(vec!["reader"], "reports.read");
    assert_eq!(
        send(&router, Method::GET, "/policy/private", Some(&missing_role)).await,
        StatusCode::FORBIDDEN
    );
    let missing_scope = token(vec!["admin"], "reports.write");
    assert_eq!(
        send(
            &router,
            Method::GET,
            "/policy/private",
            Some(&missing_scope)
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let authorized = token(vec!["admin"], "reports.read reports.write");
    assert_eq!(
        send(&router, Method::GET, "/policy/private", Some(&authorized)).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn exact_path_permits_and_method_specific_public_policies_do_not_bypass_protection() {
    let router = router();
    assert_eq!(
        send(&router, Method::GET, "/permit", None).await,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, Method::GET, "/permit/", None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&router, Method::POST, "/policy/open", None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&router, Method::GET, "/unannotated", None).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn route_matching_and_head_requests_keep_the_declared_policy() {
    let router = router();
    assert_eq!(
        send(&router, Method::HEAD, "/policy/private", None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&router, Method::GET, "/policy/private/", None).await,
        StatusCode::UNAUTHORIZED
    );

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::HEAD)
                .uri("/policy/private")
                .header(
                    "authorization",
                    format!("Bearer {}", token(vec!["admin"], "reports.read")),
                )
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn most_specific_static_policy_wins_over_a_protected_parameter_route() {
    let router = router();
    assert_eq!(
        send(&router, Method::GET, "/specific/users/42", None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&router, Method::GET, "/specific/users/me", None).await,
        StatusCode::OK
    );
    assert_eq!(
        send(&router, Method::GET, "/specific/users/me/", None).await,
        StatusCode::UNAUTHORIZED
    );
}
