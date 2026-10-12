use axum::{
    body::to_bytes,
    body::Body,
    http::{Request, StatusCode},
};
use scafra_web::{build_router_with_actuator, ActuatorConfig, ActuatorSecurity, EndpointSelection};
use tower::util::ServiceExt;

async fn test_health_check() -> bool {
    true
}

scafra_web::register_health_check!("test", test_health_check);

async fn get(path: &str) -> (StatusCode, String) {
    let response = build_router_with_actuator(&ActuatorConfig::all())
        .expect("the router should include the built-in actuator routes")
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should respond");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("response body should be readable");
    (
        status,
        String::from_utf8(body.to_vec()).expect("body should be UTF-8"),
    )
}

#[tokio::test]
async fn actuator_can_be_disabled_without_affecting_application_routes() {
    let response = build_router_with_actuator(&ActuatorConfig::default())
        .expect("the router should build with observation disabled")
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn valid_actuator_bearer_token_authorizes_http_requests() {
    let config = ActuatorConfig {
        endpoints: EndpointSelection::Pattern("metrics".to_owned()),
        security: ActuatorSecurity {
            enabled: true,
            bearer_token: Some("test-token".to_owned()),
        },
        ..ActuatorConfig::default()
    };
    let router = build_router_with_actuator(&config).expect("actuator router should build");

    let unauthorized = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let authorized = router
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
}

#[tokio::test]
async fn exposes_health_liveness_readiness_and_info_aliases() {
    for path in [
        "/health",
        "/live",
        "/ready",
        "/actuator/health",
        "/actuator/health/liveness",
        "/actuator/health/readiness",
    ] {
        let (status, body) = get(path).await;
        assert_eq!(status, StatusCode::OK, "unexpected status for {path}");
        assert_eq!(body, r#"{"status":"UP"}"#, "unexpected body for {path}");
    }

    for path in ["/info", "/actuator/info"] {
        let (status, body) = get(path).await;
        assert_eq!(status, StatusCode::OK, "unexpected status for {path}");
        assert!(body.contains(r#""name":"scafra""#));
        assert!(body.contains(&format!(r#""version":"{}""#, env!("CARGO_PKG_VERSION"))));
    }
}
