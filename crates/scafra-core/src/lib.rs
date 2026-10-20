//! Framework-neutral application primitives used by Scafra adapters.

mod errors;
mod graph;
mod lifecycle;
mod registration;

pub use errors::{
    GraphError, GraphPhase, LifecycleError, LifecycleFailure, ProviderFailure, ScafraError,
};
pub use graph::{
    GraphDescriptor, GraphEdgeDescriptor, GraphNodeDescriptor, GraphNodeKind, GraphPlan,
};
pub use lifecycle::{
    Application, ApplicationContext, LifecycleHook, LifecyclePhase, LifecycleState,
};
pub use registration::__private;
pub use registration::{
    AsyncResource, AsyncResourceSet, Bean, BeanPostProcessor, ComponentKind, ComponentMetadata,
    ComponentRegistration, OptionalExtensionRegistration, PostProcessorRegistration, Service,
};

#[cfg(test)]
mod tests;
