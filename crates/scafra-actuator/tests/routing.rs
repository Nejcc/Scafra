use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use scafra_actuator::{
    reserved_routes, router_with_health_registry, ActuatorConfig, EndpointSelection, HealthRegistry,
};
use tower::ServiceExt;

async fn get(router: &axum::Router, path: &str) -> (StatusCode, String) {
    let response = router
        .clone()
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
        String::from_utf8(body.to_vec()).expect("response should be UTF-8"),
    )
}

#[tokio::test]
async fn each_enabled_endpoint_group_installs_all_supported_aliases() {
    let config = ActuatorConfig::all();
    let router = router_with_health_registry(&config, HealthRegistry::new())
        .expect("the empty graph registry should build");
    let aliases: &[&str] = &[
        "/health",
        "/actuator/health",
        "/live",
        "/health/live",
        "/actuator/health/liveness",
        "/ready",
        "/health/ready",
        "/actuator/health/readiness",
        "/info",
        "/actuator/info",
        "/metrics",
        "/actuator/metrics",
    ];

    for path in aliases {
        assert_eq!(get(&router, path).await.0, StatusCode::OK, "{path}");
    }
    for aliases in [
        &["/health", "/actuator/health"][..],
        &["/live", "/health/live", "/actuator/health/liveness"][..],
        &["/ready", "/health/ready", "/actuator/health/readiness"][..],
        &["/info", "/actuator/info"][..],
        &["/metrics", "/actuator/metrics"][..],
    ] {
        let expected_body = get(&router, aliases[0]).await.1;
        for path in &aliases[1..] {
            let (status, body) = get(&router, path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(body, expected_body, "alias response differs for {path}");
        }
    }
}

#[tokio::test]
async fn disabled_groups_are_absent_and_reserved_routes_match_enabled_groups() {
    let config = ActuatorConfig {
        endpoints: EndpointSelection::Names(vec!["health".to_owned(), "info".to_owned()]),
        ..ActuatorConfig::default()
    };
    let router = router_with_health_registry(&config, HealthRegistry::new())
        .expect("the graph registry should build");
    let reserved = reserved_routes(&config);
    assert_eq!(
        reserved,
        [
            ("GET", "/health"),
            ("GET", "/info"),
            ("GET", "/actuator/health"),
            ("GET", "/actuator/info"),
        ]
    );

    for (path, enabled) in [
        ("/health", true),
        ("/actuator/health", true),
        ("/live", false),
        ("/health/live", false),
        ("/actuator/health/liveness", false),
        ("/ready", false),
        ("/health/ready", false),
        ("/actuator/health/readiness", false),
        ("/info", true),
        ("/actuator/info", true),
        ("/metrics", false),
        ("/actuator/metrics", false),
    ] {
        let status = get(&router, path).await.0;
        assert_eq!(
            status,
            if enabled {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            },
            "{path}"
        );
    }
}

#[tokio::test]
async fn multiple_routers_can_be_constructed_with_the_same_check_name_independently() {
    let config = ActuatorConfig {
        endpoints: EndpointSelection::Names(vec!["ready".to_owned()]),
        health: scafra_actuator::HealthConfig {
            checks: vec!["same-name".to_owned()],
            ..scafra_actuator::HealthConfig::default()
        },
        ..ActuatorConfig::default()
    };
    let mut registry_a = HealthRegistry::new();
    registry_a
        .register("same-name", || async { false })
        .expect("graph A should register its check");
    let mut registry_b = HealthRegistry::new();
    registry_b
        .register("same-name", || async { true })
        .expect("graph B should register its check");

    let router_a = router_with_health_registry(&config, registry_a)
        .expect("graph A should build with its registry");
    let router_b = router_with_health_registry(&config, registry_b)
        .expect("graph B should build with its registry");
    let (a, b) = tokio::join!(get(&router_a, "/ready"), get(&router_b, "/ready"));
    assert_eq!(a.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(b.0, StatusCode::OK);
}
