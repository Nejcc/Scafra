use crate::{
    config::ActuatorConfig,
    handlers,
    registry::{HealthRegistry, HealthRegistryError},
};
use axum::{extract::Extension, routing::get, Router};
use std::sync::Arc;

pub(crate) struct ActuatorState {
    pub(crate) config: ActuatorConfig,
    pub(crate) health_registry: HealthRegistry,
    pub(crate) health_selection_valid: bool,
}

/// Adds built-in operational routes using legacy process-wide registrations.
///
/// This infallible signature is retained for compatibility. Unknown configured
/// names produce a router whose health and readiness endpoints return `DOWN`.
/// Application startup should use [`try_router`] so configuration errors are
/// reported before serving. New applications should prefer one
/// [`HealthRegistry`] per graph and [`router_with_health_registry`].
pub fn router(config: &ActuatorConfig) -> Router {
    let registry = HealthRegistry::from_legacy_registrations();
    let valid = registry.validate(&config.health).is_ok();
    if !valid {
        tracing::error!("actuator health configuration references unknown checks");
    }
    build_router(config, registry, valid)
}

/// Adds built-in operational routes and rejects unknown configured check
/// names before returning the router.
///
/// Use this constructor at application startup when checks were registered
/// with [`crate::register_health_check!`]. The standard Scafra web builder and
/// generated application graph use this fallible path. [`router`] remains
/// available for source compatibility and fails closed at request time.
pub fn try_router(config: &ActuatorConfig) -> Result<Router, HealthRegistryError> {
    let registry = HealthRegistry::from_legacy_registrations();
    registry.validate(&config.health)?;
    Ok(build_router(config, registry, true))
}

/// Adds built-in operational routes backed by one application-owned registry.
///
/// The registry is validated and moved into the router before it is returned.
/// Build a separate registry for every graph. A check may capture graph-owned
/// state, and the router retains the registry for its serving lifetime.
pub fn router_with_health_registry(
    config: &ActuatorConfig,
    registry: HealthRegistry,
) -> Result<Router, HealthRegistryError> {
    registry.validate(&config.health)?;
    Ok(build_router(config, registry, true))
}

fn build_router(config: &ActuatorConfig, registry: HealthRegistry, valid: bool) -> Router {
    let mut router = Router::new();
    if config.is_enabled("health") {
        router = router
            .route("/health", get(handlers::health))
            .route("/actuator/health", get(handlers::health));
    }
    if config.is_enabled("live") {
        router = router
            .route("/live", get(handlers::liveness))
            .route("/health/live", get(handlers::liveness))
            .route("/actuator/health/liveness", get(handlers::liveness));
    }
    if config.is_enabled("ready") {
        router = router
            .route("/ready", get(handlers::readiness))
            .route("/health/ready", get(handlers::readiness))
            .route("/actuator/health/readiness", get(handlers::readiness));
    }
    if config.is_enabled("info") {
        router = router
            .route("/info", get(handlers::info))
            .route("/actuator/info", get(handlers::info));
    }
    if config.is_enabled("metrics") {
        router = router
            .route("/metrics", get(handlers::metrics))
            .route("/actuator/metrics", get(handlers::metrics));
    }
    router.layer(Extension(Arc::new(ActuatorState {
        config: config.clone(),
        health_registry: registry,
        health_selection_valid: valid,
    })))
}

/// Returns the built-in paths that must not be shadowed by application routes.
pub fn reserved_routes(config: &ActuatorConfig) -> Vec<(&'static str, &'static str)> {
    ROUTES
        .iter()
        .copied()
        .filter(|(_, path)| match *path {
            "/health" | "/actuator/health" => config.is_enabled("health"),
            "/live" | "/health/live" | "/actuator/health/liveness" => config.is_enabled("live"),
            "/ready" | "/health/ready" | "/actuator/health/readiness" => config.is_enabled("ready"),
            "/info" | "/actuator/info" => config.is_enabled("info"),
            "/metrics" | "/actuator/metrics" => config.is_enabled("metrics"),
            _ => false,
        })
        .collect()
}

const ROUTES: [(&str, &str); 12] = [
    ("GET", "/health"),
    ("GET", "/health/live"),
    ("GET", "/health/ready"),
    ("GET", "/live"),
    ("GET", "/ready"),
    ("GET", "/info"),
    ("GET", "/actuator/health"),
    ("GET", "/actuator/health/liveness"),
    ("GET", "/actuator/health/readiness"),
    ("GET", "/actuator/info"),
    ("GET", "/metrics"),
    ("GET", "/actuator/metrics"),
];
