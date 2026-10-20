use scafra_foundation::{Phase, PhaseMetadata, ShutdownPolicy, ShutdownReason};
use std::future::Future;

use crate::GraphError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    Bean,
    Configuration,
    Service,
    Controller,
}

/// Small, static metadata record for a framework component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentMetadata {
    pub name: &'static str,
    pub kind: ComponentKind,
}

/// Link-time component metadata emitted by component macros.
pub struct ComponentRegistration {
    pub name: &'static str,
    pub kind: ComponentKind,
}

/// A framework extension that is linked into the application explicitly.
pub struct OptionalExtensionRegistration {
    pub name: &'static str,
    pub start: fn(),
}

inventory::collect!(OptionalExtensionRegistration);

inventory::collect!(ComponentRegistration);

/// A typed extension point invoked around component initialization.
pub trait BeanPostProcessor: Send + Sync + 'static {
    fn before_initialization(
        &self,
        _bean_name: &str,
        _metadata: &ComponentMetadata,
    ) -> Result<(), String> {
        Ok(())
    }

    fn after_initialization(
        &self,
        _bean_name: &str,
        _metadata: &ComponentMetadata,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Static registration record emitted by `#[post_processor]`.
pub struct PostProcessorRegistration {
    pub name: &'static str,
    pub create: fn() -> Box<dyn BeanPostProcessor>,
}

impl PostProcessorRegistration {
    /// Derives the stable startup metadata for this compatibility-preserving
    /// registration shape.
    pub const fn metadata(&self) -> PhaseMetadata {
        PhaseMetadata::new(Phase::Startup, self.name, 0, "inventory::post_processor")
    }
}

inventory::collect!(PostProcessorRegistration);

/// Optional marker trait for application-defined bean values.
///
/// Typed graph composition does not require this marker. The `#[bean]` macro
/// intentionally does not implement it: a macro cannot reliably distinguish a
/// local type from a foreign type imported under a local name, and Rust's
/// orphan rules forbid implementing Scafra's trait for a foreign type.
/// Applications that use `Bean` as their own bound can implement it for local
/// types.
pub trait Bean: Send + Sync + 'static {}

/// A resource created by an asynchronous graph provider.
///
/// Applications implement this trait on an application-owned type, usually a
/// newtype around a third-party client or pool. `shutdown` must cooperatively
/// stop work owned by the resource and return after that work has drained.
/// The supplied policy lets the implementation apply its grace period. The
/// framework does not forcibly cancel arbitrary provider or cleanup futures.
pub trait AsyncResource: Send + Sync + 'static {
    /// The typed error returned when resource teardown fails.
    type ShutdownError: std::error::Error + Send + Sync + 'static;

    /// Prevents the resource from accepting new owned work when shutdown
    /// begins. Implementations should make this operation prompt and
    /// idempotent; the default is suitable for resources without task loops.
    fn request_shutdown(&self, _reason: ShutdownReason) {}

    /// Requests cooperative shutdown and waits for owned work to finish.
    fn shutdown<'a>(
        &'a self,
        reason: ShutdownReason,
        policy: ShutdownPolicy,
    ) -> impl Future<Output = Result<(), Self::ShutdownError>> + Send + 'a;
}

/// The generated, statically typed set of resources retained for an
/// application's lifetime.
///
/// Implementations shut resources down in reverse dependency order and keep
/// attempting cleanup after an individual failure.
pub trait AsyncResourceSet: Send + Sync + 'static {
    /// Starts cooperative shutdown for every owned resource. The generated
    /// implementation calls resources in reverse dependency order.
    fn request_shutdown(&self, _reason: ShutdownReason) {}

    /// Shuts down all resources owned by this set.
    fn shutdown<'a>(
        &'a self,
        reason: ShutdownReason,
        policy: ShutdownPolicy,
    ) -> impl Future<Output = Result<(), GraphError>> + Send + 'a;
}

impl AsyncResourceSet for () {
    async fn shutdown(
        &self,
        _reason: ShutdownReason,
        _policy: ShutdownPolicy,
    ) -> Result<(), GraphError> {
        Ok(())
    }
}

#[doc(hidden)]
pub mod __private {
    pub use inventory;
}

/// Marker trait implemented by `#[service]` components.
pub trait Service: Send + Sync + 'static {}
