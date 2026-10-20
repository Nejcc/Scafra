//! Axum/Tokio integration for Scafra.

mod errors;
mod json;
mod registration;
mod routing;
mod server;

pub use axum;
pub use scafra_actuator;
pub use scafra_core;
pub use tower;

/// Internal re-exports used by generated code. They are public only so a
/// downstream application does not need to depend on Scafra's implementation
/// crates directly.
#[doc(hidden)]
pub mod __private {
    pub use inventory;
}

pub use errors::{AppError, ServerError, WebError};
pub use json::JsonBody;
pub use registration::{ControllerPrefix, ControllerRegistration, ControllerRoutes, RouteMetadata};
pub use routing::{
    build_router, build_router_with_actuator, build_router_with_actuator_and_health_registry,
    build_router_with_actuator_and_health_registry_and_security,
    build_router_with_actuator_and_security, default_controller_metadata, finish_router,
    join_paths, register_default_controllers, validate_routes,
};
pub use scafra_actuator::register_health_check;
pub use scafra_actuator::{
    ActuatorConfig, ActuatorSecurity, EndpointSelection, HealthCheckFuture,
    HealthCheckRegistration, HealthConfig, HealthRegistry, HealthRegistryError,
};
pub use scafra_security::{
    AuthorizationMode, AuthorizationPolicy, BasicAuthConfig, ControllerAuthorizationRegistration,
    JwtConfig, RouteAuthorizationMetadata, SecurityConfig,
};
pub use server::{
    run, run_on, run_on_with_log_level, serve_on, serve_on_with_actuator, serve_on_with_policy,
    serve_on_with_policy_and_actuator, serve_on_with_policy_and_actuator_and_security,
    serve_on_with_policy_and_actuator_and_security_and_request_timeout,
    serve_on_with_policy_and_actuator_and_security_and_request_timeout_and_shutdown_callback,
    serve_on_with_shutdown,
    serve_router_on_with_policy_and_actuator_and_security_and_request_timeout,
    serve_router_on_with_policy_and_actuator_and_security_and_request_timeout_and_shutdown_callback,
    shutdown_channel, ServerOutcome, ShutdownFuture, ShutdownHandle, ShutdownRequestError,
};

#[cfg(test)]
mod tests;
