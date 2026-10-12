//! Configurable operational endpoints for Scafra applications.
//!
//! [`HealthRegistry`] provides graph-scoped checks. Build and populate one
//! registry for each application, then move it into
//! [`router_with_health_registry`]. The router owns the immutable registry for
//! its lifetime, and health checks may capture owned application state such as
//! an `Arc` to a graph service.
//!
//! [`register_health_check!`] and [`HealthCheckRegistration`] remain available
//! for compatibility. Those registrations are process-wide, have no access to
//! an application graph, and are included only by [`router`]. They are suitable
//! for simple process-wide checks, not per-application dependencies.

mod config;
mod handlers;
mod health;
mod registry;
mod routing;
mod security;

pub use config::{ActuatorConfig, ActuatorSecurity, EndpointSelection, HealthConfig};
pub use health::HealthCheckFuture;
pub use registry::HealthCheckRegistration;
pub use registry::{HealthRegistry, HealthRegistryError};
pub use routing::{reserved_routes, router, router_with_health_registry};

pub use inventory;

/// Registers a legacy process-wide asynchronous actuator health check.
///
/// This compatibility macro cannot capture application-owned services and its
/// registrations are visible to every router built with [`router`]. Use
/// [`HealthRegistry::register`] and [`router_with_health_registry`] for checks
/// scoped to one application graph. A selected check is subject to
/// [`HealthConfig::check_timeout_ms`].
///
/// ```ignore
/// async fn process_is_ready() -> bool {
///     true
/// }
/// scafra::register_health_check!("process", process_is_ready);
/// ```
#[macro_export]
macro_rules! register_health_check {
    ($name:literal, $check:path) => {
        $crate::inventory::submit! {
            $crate::HealthCheckRegistration {
                name: $name,
                check: || Box::pin($check()),
            }
        }
    };
}
