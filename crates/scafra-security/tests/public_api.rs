use scafra_security::{
    layer, layer_with_route_policies, AuthorizationMode, AuthorizationPolicy, BasicAuthConfig,
    ControllerAuthorizationRegistration, JwtConfig, RouteAuthorizationMetadata, SecurityConfig,
};

const REGISTERED_ROUTES: &[RouteAuthorizationMetadata] = &[RouteAuthorizationMetadata {
    controller: "external_controller",
    method: "GET",
    prefix: "/external",
    path: "/route",
    controller_policy: AuthorizationPolicy::inherit(),
    route_policy: AuthorizationPolicy::public(),
}];

inventory::submit! {
    ControllerAuthorizationRegistration {
        controller: "external_controller",
        routes: REGISTERED_ROUTES,
    }
}

#[test]
fn public_security_api_remains_available_from_the_crate_root() {
    let config = SecurityConfig {
        basic: BasicAuthConfig::default(),
        jwt: JwtConfig::default(),
        ..SecurityConfig::default()
    };
    let registration = ControllerAuthorizationRegistration {
        controller: "test",
        routes: REGISTERED_ROUTES,
    };
    let metadata = registration.routes[0];
    assert_eq!(metadata.controller_policy.mode, AuthorizationMode::Inherit);
    assert_eq!(metadata.route_policy, AuthorizationPolicy::public());

    let router = axum::Router::new();
    let _router = layer(router, config.clone());
    let _router = layer_with_route_policies(configured_router(), config, vec![metadata]);
}

fn configured_router() -> axum::Router {
    axum::Router::new()
}
