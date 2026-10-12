use std::collections::HashMap;
use std::time::Duration;

use axum::{extract::DefaultBodyLimit, http::StatusCode, Router};
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};

use crate::{errors::WebError, registration::ControllerRegistration};
use scafra_actuator::{ActuatorConfig, HealthRegistry};

/// Joins a controller prefix and method path while preserving Axum route
/// syntax, including path parameters such as {id}.
pub fn join_paths(prefix: &str, path: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    match (prefix.is_empty(), path.is_empty()) {
        (true, true) => "/".to_owned(),
        (true, false) => format!("/{path}"),
        (false, true) => prefix.to_owned(),
        (false, false) => format!("{prefix}/{path}"),
    }
}

/// Builds the application router from all statically linked controllers.
pub fn build_router() -> Result<Router, WebError> {
    build_router_with_actuator(&ActuatorConfig::default())
}

/// Builds the application router with configurable operational endpoints and
/// legacy process-wide health registrations.
///
/// Use [`build_router_with_actuator_and_health_registry`] to pass checks owned
/// by one application graph.
pub fn build_router_with_actuator(actuator: &ActuatorConfig) -> Result<Router, WebError> {
    build_router_with_actuator_and_security(actuator, &scafra_security::SecurityConfig::default())
}

/// Builds the application router with legacy process-wide health registrations
/// and the supplied application security configuration.
///
/// Use [`build_router_with_actuator_and_health_registry_and_security`] to pass
/// checks owned by one application graph.
pub fn build_router_with_actuator_and_security(
    actuator: &ActuatorConfig,
    security: &scafra_security::SecurityConfig,
) -> Result<Router, WebError> {
    build_router_with_timeout(actuator, security, None)
}

/// Builds the default application router with checks owned by one graph.
pub fn build_router_with_actuator_and_health_registry(
    actuator: &ActuatorConfig,
    registry: HealthRegistry,
) -> Result<Router, WebError> {
    build_router_with_actuator_and_health_registry_and_security(
        actuator,
        registry,
        &scafra_security::SecurityConfig::default(),
    )
}

/// Builds the default application router with graph-scoped health checks and
/// the supplied application security configuration.
pub fn build_router_with_actuator_and_health_registry_and_security(
    actuator: &ActuatorConfig,
    registry: HealthRegistry,
    security: &scafra_security::SecurityConfig,
) -> Result<Router, WebError> {
    let router = scafra_actuator::router_with_health_registry(actuator, registry)?;
    build_router_with_actuator_router(router, actuator, security, None)
}

pub(crate) fn build_router_with_timeout(
    actuator: &ActuatorConfig,
    security: &scafra_security::SecurityConfig,
    request_timeout: Option<Duration>,
) -> Result<Router, WebError> {
    let router = scafra_actuator::router(actuator);
    build_router_with_actuator_router(router, actuator, security, request_timeout)
}

fn build_router_with_actuator_router(
    router: Router,
    actuator: &ActuatorConfig,
    security: &scafra_security::SecurityConfig,
    request_timeout: Option<Duration>,
) -> Result<Router, WebError> {
    let routes = default_controller_metadata(&[]);
    validate_routes(&routes, actuator)?;
    let router = register_default_controllers(router, &[]);
    finish_router(router, &routes, actuator, security, request_timeout)
}

/// Applies standard route validation and middleware to a router whose
/// controller instances were constructed by typed application composition.
pub fn finish_router(
    router: Router,
    routes: &[crate::registration::RouteMetadata],
    actuator: &ActuatorConfig,
    security: &scafra_security::SecurityConfig,
    request_timeout: Option<Duration>,
) -> Result<Router, WebError> {
    validate_routes(routes, actuator)?;
    let route_policies = authorization_metadata_for(routes);
    validate_security_config(security, &route_policies)?;
    let router = router.layer(DefaultBodyLimit::max(1024 * 1024));

    let router =
        scafra_security::layer_with_route_policies(router, security.clone(), route_policies);
    let router = with_request_timeout(router, request_timeout);
    Ok(router.layer(TraceLayer::new_for_http().make_span_with(
        |request: &axum::http::Request<_>| {
            tracing::debug_span!(
                "request",
                method = %request.method(),
                version = ?request.version(),
            )
        },
    )))
}

fn authorization_metadata_for(
    routes: &[crate::registration::RouteMetadata],
) -> Vec<scafra_security::RouteAuthorizationMetadata> {
    inventory::iter::<scafra_security::ControllerAuthorizationRegistration>()
        .flat_map(|registration| registration.routes.iter().copied())
        .filter(|policy| {
            routes.iter().any(|route| {
                route.controller == policy.controller
                    && route.method == policy.method
                    && route.prefix == policy.prefix
                    && route.path == policy.path
            })
        })
        .collect()
}

fn validate_security_config(
    security: &scafra_security::SecurityConfig,
    policies: &[scafra_security::RouteAuthorizationMetadata],
) -> Result<(), WebError> {
    if security.enabled {
        security.validate().map_err(security_configuration_error)?;
    }
    let has_protected_route = policies.iter().any(|policy| {
        if policy.route_policy.mode == scafra_security::AuthorizationMode::Public {
            false
        } else if policy.route_policy.mode == scafra_security::AuthorizationMode::Protected {
            true
        } else {
            policy.controller_policy.mode == scafra_security::AuthorizationMode::Protected
        }
    });
    if has_protected_route && !security.enabled {
        return Err(security_configuration_error(
            "security must be enabled when a controller or route requires authentication",
        ));
    }
    Ok(())
}

fn security_configuration_error(reason: &'static str) -> WebError {
    WebError::Server(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("invalid security configuration: {reason}"),
    ))
}

/// Validates route metadata before any router merge occurs.
pub fn validate_routes(
    routes: &[crate::registration::RouteMetadata],
    actuator: &ActuatorConfig,
) -> Result<(), WebError> {
    let mut seen = HashMap::<(String, String), &'static str>::new();
    for (method, path) in scafra_actuator::reserved_routes(actuator) {
        seen.insert(
            ((*method).to_owned(), (*path).to_owned()),
            "scafra-actuator",
        );
    }
    for route in routes {
        let key = (
            route.method.to_owned(),
            join_paths(route.prefix, route.path),
        );
        if let Some(first) = seen.insert(key.clone(), route.controller) {
            return Err(WebError::DuplicateRoute {
                method: key.0,
                path: key.1,
                first,
                second: route.controller,
            });
        }
    }
    Ok(())
}

/// Registers statically linked route adapters that were not included in typed
/// graph composition. Handwritten adapters can retain legacy behavior here;
/// generated controllers are registered from constructed graph instances.
pub fn register_default_controllers(mut router: Router, composed_controllers: &[&str]) -> Router {
    for registration in inventory::iter::<ControllerRegistration>() {
        if composed_controllers.contains(&registration.controller) {
            continue;
        }
        router = (registration.register)(router);
    }
    router
}

/// Returns route metadata for linked controllers, excluding controllers that
/// typed composition will register itself.
pub fn default_controller_metadata(
    composed_controllers: &[&str],
) -> Vec<crate::registration::RouteMetadata> {
    inventory::iter::<ControllerRegistration>()
        .filter(|registration| !composed_controllers.contains(&registration.controller))
        .flat_map(|registration| registration.routes.iter().copied())
        .collect()
}

pub(crate) fn with_request_timeout(router: Router, request_timeout: Option<Duration>) -> Router {
    match request_timeout {
        Some(timeout) => router.layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            timeout,
        )),
        None => router,
    }
}
