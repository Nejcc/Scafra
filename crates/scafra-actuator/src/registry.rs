use crate::{config::HealthConfig, health, HealthCheckFuture};
use std::{collections::HashSet, error::Error, fmt, future::Future};

/// A legacy health check registered for every compatibility router in this
/// process. Prefer [`HealthRegistry::register`] for application-owned checks.
pub struct HealthCheckRegistration {
    pub name: &'static str,
    pub check: fn() -> HealthCheckFuture,
}

inventory::collect!(HealthCheckRegistration);

pub(crate) type HealthCheckFactory = Box<dyn Fn() -> HealthCheckFuture + Send + Sync + 'static>;

pub(crate) struct RegisteredCheck {
    pub(crate) name: String,
    pub(crate) check: HealthCheckFactory,
}

/// Health checks owned by one application router.
///
/// Populate the registry before building the router. The router takes
/// ownership and evaluates this fixed set for its entire lifetime, so later
/// requests cannot observe registrations from another graph. Checks may own
/// graph state through captured values, commonly an `Arc<T>`.
///
/// Names must be non-empty and unique within a registry. Registration order is
/// preserved for sequential evaluation. The registry is consumed by
/// [`crate::router_with_health_registry`], which prevents registrations after
/// the router starts serving.
///
/// This ownership check is enforced by Rust: after router construction, the
/// registry has moved into the router and cannot be mutated.
///
/// ```compile_fail
/// use scafra_actuator::{router_with_health_registry, ActuatorConfig, HealthRegistry};
///
/// let mut registry = HealthRegistry::new();
/// let _router = router_with_health_registry(&ActuatorConfig::all(), registry).unwrap();
/// registry.register("late", || async { true }).unwrap();
/// ```
pub struct HealthRegistry {
    checks: Vec<RegisteredCheck>,
}

impl fmt::Debug for HealthRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HealthRegistry")
            .field("check_count", &self.checks.len())
            .finish()
    }
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthRegistry {
    /// Creates an empty registry for one application graph.
    pub fn new() -> Self {
        Self { checks: Vec::new() }
    }

    /// Adds an async check to this application's registry.
    ///
    /// The factory is called once per evaluation. It must be safe to invoke
    /// concurrently because independent requests may evaluate a router at the
    /// same time. Capture `Arc` values when the check needs owned application
    /// services.
    pub fn register<F, Fut>(
        &mut self,
        name: impl Into<String>,
        check: F,
    ) -> Result<(), HealthRegistryError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = bool> + Send + 'static,
    {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(HealthRegistryError::EmptyCheckName);
        }
        if self
            .checks
            .iter()
            .any(|registration| registration.name == name)
        {
            return Err(HealthRegistryError::DuplicateCheckName(name));
        }

        self.checks.push(RegisteredCheck {
            name,
            check: Box::new(move || Box::pin(check())),
        });
        Ok(())
    }

    /// Validates configured check names against this registry.
    ///
    /// `*` selects all checks. Unknown names are counted in the error without
    /// including configured values in its diagnostic.
    pub fn validate(&self, config: &HealthConfig) -> Result<(), HealthRegistryError> {
        let mut unknown_count = 0;
        let mut visited = HashSet::new();
        for name in config.checks.iter().filter(|name| name.as_str() != "*") {
            if visited.insert(name.as_str()) && !self.checks.iter().any(|check| check.name == *name)
            {
                unknown_count += 1;
            }
        }
        if unknown_count == 0 {
            Ok(())
        } else {
            Err(HealthRegistryError::UnknownConfiguredChecks {
                count: unknown_count,
            })
        }
    }

    /// Evaluates the selected checks and returns `false` on invalid
    /// configuration, a `false` result, timeout, or an unwinding panic.
    pub async fn is_healthy(&self, config: &HealthConfig) -> bool {
        if self.validate(config).is_err() {
            tracing::error!("actuator health configuration references unknown checks");
            return false;
        }
        health::evaluate(self, config).await
    }

    pub(crate) fn selected_checks<'a>(
        &'a self,
        config: &'a HealthConfig,
    ) -> impl Iterator<Item = &'a RegisteredCheck> + 'a {
        self.checks.iter().filter(|registration| {
            config.checks.is_empty()
                || config
                    .checks
                    .iter()
                    .any(|name| name == "*" || name == &registration.name)
        })
    }

    pub(crate) fn from_legacy_registrations() -> Self {
        let checks = inventory::iter::<HealthCheckRegistration>
            .into_iter()
            .map(|registration| {
                let check = registration.check;
                RegisteredCheck {
                    name: registration.name.to_owned(),
                    check: Box::new(move || check()),
                }
            })
            .collect();
        Self { checks }
    }
}

/// An invalid application-scoped health registry or selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthRegistryError {
    /// Registration names cannot be empty or whitespace-only.
    EmptyCheckName,
    /// A registry cannot contain two checks with the same name.
    DuplicateCheckName(String),
    /// A configured selection contains names absent from this registry.
    /// The number is reported without exposing configuration values.
    UnknownConfiguredChecks { count: usize },
}

impl fmt::Display for HealthRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCheckName => formatter.write_str("health check names cannot be empty"),
            Self::DuplicateCheckName(name) => {
                write!(
                    formatter,
                    "health check name is registered more than once: {name}"
                )
            }
            Self::UnknownConfiguredChecks { count } => write!(
                formatter,
                "actuator health configuration references {count} unknown check name(s)"
            ),
        }
    }
}

impl Error for HealthRegistryError {}
