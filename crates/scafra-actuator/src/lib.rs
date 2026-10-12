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
//!
//! # Public API and behavior
//!
//! [`ActuatorConfig`] selects endpoint groups, health-check names, per-check
//! timeout, and optional bearer-token protection. Endpoint groups are disabled
//! by default; [`ActuatorConfig::all`] enables health, liveness, readiness,
//! info, and metrics. Configuration keeps its existing serde representation,
//! while bearer tokens are omitted during serialization and redacted from
//! debug output.
//!
//! [`HealthRegistry::register`] accepts graph-owned asynchronous checks. Names
//! must be non-empty and unique within one registry; duplicate names are
//! rejected at registration. Graph-scoped router construction rejects unknown
//! selected names, including unknown names listed alongside `*`. An empty
//! selection or `*` selects every check; an empty registry is healthy.
//! Registries are validated and moved into the router at construction, so
//! registrations cannot change during serving and separate routers do not
//! share graph-owned health state.
//!
//! Checks run sequentially and each receives its own timeout. A check that
//! returns `false`, times out, or unwinds while its future is created or polled
//! makes aggregate health and readiness return `DOWN` without returning the
//! panic payload. The actuator suppresses the process panic hook only during
//! health-check creation and polling, while the previously installed hook
//! continues to receive other panics. A later process call to `set_hook` can
//! replace this wrapper. Liveness always returns `UP` after authorization and
//! never executes dependency checks.
//!
//! When actuator security is enabled, every installed actuator route requires
//! the exact configured bearer token. Missing, invalid, or blank tokens fail
//! closed. Startup configuration validation also rejects blank configured
//! tokens when protection is enabled.
//!
//! # Migration from static checks
//!
//! Existing `register_health_check!` registrations remain available through
//! [`router`] and the Scafra compatibility router builders. They remain
//! process-wide and are not copied into a [`HealthRegistry`]. To give a check
//! access to graph-owned services, create a registry while composing that
//! graph, register checks that capture owned service handles, then pass the
//! registry to [`router_with_health_registry`] or the corresponding web/facade
//! builder. Each router owns only the registry passed to that construction.
//!
//! The crate is organized by responsibility: `config` owns public settings,
//! `registry` owns registrations and validation, `health` evaluates checks,
//! `security` authorizes requests, `handlers` produces endpoint responses, and
//! `routing` composes routes and reports reserved paths. These modules remain
//! private; public items are re-exported from the crate root.

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
