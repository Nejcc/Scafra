use crate::lifecycle::{LifecyclePhase, LifecycleState};
use scafra_foundation::{Phase, PhaseMetadata, ShutdownPolicy, ShutdownReason};
use std::{error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphPhase {
    Validation,
    Construction,
}

/// Framework-level errors that are independent of a transport adapter.
#[derive(Debug, thiserror::Error)]
pub enum ScafraError {
    #[error("application lifecycle is invalid: cannot {action} while state is {state:?}")]
    InvalidLifecycle {
        action: &'static str,
        state: LifecycleState,
    },

    #[error("{phase:?} hook `{name}` failed: {message}")]
    Hook {
        phase: LifecyclePhase,
        name: &'static str,
        message: String,
    },

    #[error("bean post-processor `{name}` failed during {phase}: {message}")]
    BeanPostProcessor {
        name: &'static str,
        phase: &'static str,
        message: String,
    },

    #[error(transparent)]
    Lifecycle(#[from] LifecycleError),
}

/// The typed failure of one lifecycle participant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleFailure {
    phase: Phase,
    participant: PhaseMetadata,
    message: String,
}

impl LifecycleFailure {
    pub(crate) fn new(
        phase: Phase,
        participant: PhaseMetadata,
        message: impl Into<String>,
    ) -> Self {
        Self {
            phase,
            participant,
            message: message.into(),
        }
    }

    /// Returns the execution phase in which the participant failed.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Returns the stable metadata of the participant that failed.
    pub fn participant(&self) -> PhaseMetadata {
        self.participant
    }

    /// Returns the participant metadata.
    pub fn metadata(&self) -> PhaseMetadata {
        self.participant()
    }

    /// Returns the participant's failure message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for LifecycleFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} participant `{}` at {} failed: {}",
            self.phase.name(),
            self.participant.name(),
            self.participant.source(),
            self.message
        )
    }
}

/// A structured failure from application lifecycle coordination.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleError {
    #[error("application startup failed: {original}; rollback failures: {rollback_failures:?}")]
    StartupFailure {
        reason: ShutdownReason,
        original: LifecycleFailure,
        rollback_failures: Vec<LifecycleFailure>,
    },

    #[error("application shutdown failed ({reason:?}): {failures:?}")]
    ShutdownFailure {
        reason: ShutdownReason,
        policy: ShutdownPolicy,
        failures: Vec<LifecycleFailure>,
    },

    #[error(
        "invalid lifecycle metadata for `{}` at {}: expected startup participant `{}`",
        metadata.name(),
        metadata.source(),
        expected_name
    )]
    InvalidMetadata {
        metadata: PhaseMetadata,
        expected_phase: Phase,
        expected_name: &'static str,
    },

    #[error(
        "duplicate lifecycle metadata key (order {}, name `{}`, source `{}`) conflicts with source `{}`",
        first.order(),
        first.name(),
        first.source(),
        duplicate.source()
    )]
    DuplicateMetadata {
        first: PhaseMetadata,
        duplicate: PhaseMetadata,
    },
}

impl LifecycleError {
    /// Returns the original startup failure, when this is a startup error.
    pub fn original(&self) -> Option<&LifecycleFailure> {
        match self {
            Self::StartupFailure { original, .. } => Some(original),
            _ => None,
        }
    }

    /// Returns the original startup failure, when this is a startup error.
    pub fn original_failure(&self) -> Option<&LifecycleFailure> {
        self.original()
    }

    /// Returns all rollback or shutdown cleanup failures.
    pub fn failures(&self) -> &[LifecycleFailure] {
        match self {
            Self::StartupFailure {
                rollback_failures, ..
            } => rollback_failures,
            Self::ShutdownFailure { failures, .. } => failures,
            Self::InvalidMetadata { .. } | Self::DuplicateMetadata { .. } => &[],
        }
    }

    /// Returns all failures produced while cleaning up lifecycle work.
    pub fn cleanup_failures(&self) -> &[LifecycleFailure] {
        self.failures()
    }

    /// Returns the shutdown reason carried by an aggregate lifecycle failure.
    pub fn reason(&self) -> Option<ShutdownReason> {
        match self {
            Self::StartupFailure { reason, .. } | Self::ShutdownFailure { reason, .. } => {
                Some(*reason)
            }
            Self::InvalidMetadata { .. } | Self::DuplicateMetadata { .. } => None,
        }
    }

    /// Returns the shutdown policy carried by a shutdown failure.
    pub fn policy(&self) -> Option<ShutdownPolicy> {
        match self {
            Self::ShutdownFailure { policy, .. } => Some(*policy),
            Self::StartupFailure { .. }
            | Self::InvalidMetadata { .. }
            | Self::DuplicateMetadata { .. } => None,
        }
    }
}

impl ScafraError {
    /// Returns the structured lifecycle error, when present.
    pub fn lifecycle(&self) -> Option<&LifecycleError> {
        match self {
            Self::Lifecycle(error) => Some(error),
            Self::InvalidLifecycle { .. } | Self::Hook { .. } | Self::BeanPostProcessor { .. } => {
                None
            }
        }
    }

    /// Returns the original startup failure, when present.
    pub fn original_lifecycle_failure(&self) -> Option<&LifecycleFailure> {
        self.lifecycle().and_then(LifecycleError::original)
    }

    /// Returns every lifecycle cleanup failure, when present.
    pub fn lifecycle_cleanup_failures(&self) -> &[LifecycleFailure] {
        self.lifecycle()
            .map_or(&[], LifecycleError::cleanup_failures)
    }
}

/// A graph validation or provider-construction failure.
#[derive(Debug)]
pub enum GraphError {
    DuplicateOutput {
        output: &'static str,
        first_provider: &'static str,
        first_source: &'static str,
        second_provider: &'static str,
        second_source: &'static str,
    },
    UnknownConsumer {
        consumer: &'static str,
        dependency: &'static str,
        source: &'static str,
    },
    MissingDependency {
        consumer: &'static str,
        dependency: &'static str,
        source: &'static str,
    },
    OwnedDependency {
        dependency: &'static str,
        first_consumer: &'static str,
        first_source: &'static str,
        second_consumer: &'static str,
        second_source: &'static str,
    },
    Cycle {
        path: Vec<&'static str>,
        sources: Vec<&'static str>,
    },
    ProviderFailure(ProviderFailure),
}

impl GraphError {
    pub fn provider_failure(
        provider: &'static str,
        phase: GraphPhase,
        source: &'static str,
        error: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self::ProviderFailure(ProviderFailure::new(provider, phase, source, error))
    }

    /// Returns the original provider failure when initialization required
    /// cleanup after earlier providers had succeeded.
    pub fn original_provider_failure(&self) -> Option<&ProviderFailure> {
        match self {
            Self::ProviderFailure(failure) => failure.original_provider_failure(),
            _ => None,
        }
    }

    /// Returns provider cleanup failures collected during startup rollback or
    /// application shutdown.
    pub fn cleanup_failures(&self) -> &[ProviderFailure] {
        match self {
            Self::ProviderFailure(failure) => failure.cleanup_failures(),
            _ => &[],
        }
    }

    /// Preserves a provider initialization error together with every failed
    /// rollback operation while retaining the existing graph error variant.
    pub fn startup_failure(
        original: ProviderFailure,
        cleanup_failures: Vec<ProviderFailure>,
    ) -> Self {
        Self::ProviderFailure(ProviderFailure::startup_failure(original, cleanup_failures))
    }

    /// Reports all resource shutdown failures while retaining the existing
    /// graph error variant.
    pub fn shutdown_failure(failures: Vec<ProviderFailure>) -> Self {
        Self::ProviderFailure(ProviderFailure::shutdown_failure(failures))
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateOutput {
                output,
                first_provider,
                first_source,
                second_provider,
                second_source,
            } => write!(
                formatter,
                "duplicate graph output `{output}` from `{first_provider}` at {first_source} and `{second_provider}` at {second_source}"
            ),
            Self::UnknownConsumer {
                consumer,
                dependency,
                source,
            } => write!(
                formatter,
                "graph edge consumer `{consumer}` is not declared while resolving `{dependency}` at {source}"
            ),
            Self::MissingDependency {
                consumer,
                dependency,
                source,
            } => write!(
                formatter,
                "missing graph dependency `{dependency}` required by `{consumer}` at {source}"
            ),
            Self::OwnedDependency {
                dependency,
                first_consumer,
                first_source,
                second_consumer,
                second_source,
            } if first_consumer == second_consumer => write!(
                formatter,
                "graph dependency `{dependency}` is consumed more than once by `{first_consumer}` at {first_source} and {second_source}"
            ),
            Self::OwnedDependency {
                dependency,
                first_consumer,
                first_source,
                second_consumer,
                second_source,
            } => write!(
                formatter,
                "graph dependency `{dependency}` has multiple consumers: `{first_consumer}` at {first_source} and `{second_consumer}` at {second_source}"
            ),
            Self::Cycle { path, sources } => {
                write!(formatter, "graph contains a dependency cycle: {}", path.join(" -> "))?;
                if !sources.is_empty() {
                    write!(formatter, " (declarations: {})", sources.join(", "))?;
                }
                Ok(())
            }
            Self::ProviderFailure(failure) => failure.fmt(formatter),
        }
    }
}

impl Error for GraphError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ProviderFailure(failure) => Some(failure),
            _ => None,
        }
    }
}

/// Context for a provider error. The owned source is deliberately omitted from
/// `Display` and `Debug`; callers can inspect it through the error chain.
pub struct ProviderFailure {
    provider: &'static str,
    phase: GraphPhase,
    source: &'static str,
    error: Box<dyn Error + Send + Sync + 'static>,
    operation: &'static str,
    original: Option<Box<ProviderFailure>>,
    cleanup_failures: Vec<ProviderFailure>,
}

impl ProviderFailure {
    pub fn new(
        provider: &'static str,
        phase: GraphPhase,
        source: &'static str,
        error: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            provider,
            phase,
            source,
            error: Box::new(error),
            operation: "construction",
            original: None,
            cleanup_failures: Vec::new(),
        }
    }

    /// Records one resource shutdown failure.
    pub fn cleanup_failure(
        provider: &'static str,
        source: &'static str,
        error: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            provider,
            phase: GraphPhase::Construction,
            source,
            error: Box::new(error),
            operation: "cleanup",
            original: None,
            cleanup_failures: Vec::new(),
        }
    }

    fn startup_failure(original: ProviderFailure, cleanup_failures: Vec<ProviderFailure>) -> Self {
        Self {
            provider: original.provider,
            phase: original.phase,
            source: original.source,
            error: Box::new(ProviderFailureSummary("startup rollback")),
            operation: "startup rollback",
            original: Some(Box::new(original)),
            cleanup_failures,
        }
    }

    fn shutdown_failure(cleanup_failures: Vec<ProviderFailure>) -> Self {
        Self {
            provider: "application resources",
            phase: GraphPhase::Construction,
            source: "generated resource set",
            error: Box::new(ProviderFailureSummary("resource shutdown")),
            operation: "resource shutdown",
            original: None,
            cleanup_failures,
        }
    }

    pub fn provider(&self) -> &'static str {
        self.provider
    }

    pub fn phase(&self) -> GraphPhase {
        self.phase
    }

    pub fn source_location(&self) -> &'static str {
        self.source
    }

    /// Returns whether this error occurred during provider construction,
    /// startup rollback, or application shutdown.
    pub fn operation(&self) -> &'static str {
        self.operation
    }

    /// Returns the provider failure that caused startup rollback.
    pub fn original_provider_failure(&self) -> Option<&ProviderFailure> {
        self.original.as_deref()
    }

    /// Returns cleanup failures collected after the original initialization
    /// failure, or during application shutdown.
    pub fn cleanup_failures(&self) -> &[ProviderFailure] {
        &self.cleanup_failures
    }
}

impl fmt::Debug for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderFailure")
            .field("provider", &self.provider)
            .field("phase", &self.phase)
            .field("source", &self.source)
            .field("operation", &self.operation)
            .field("cleanup_failures", &self.cleanup_failures)
            .field("error", &"<redacted; inspect Error::source explicitly>")
            .finish()
    }
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.operation {
            "startup rollback" => write!(
                formatter,
                "application resource provider `{}` failed during startup at {} ({} cleanup failure(s))",
                self.provider,
                self.source,
                self.cleanup_failures.len()
            ),
            "resource shutdown" => write!(
                formatter,
                "application resource shutdown had {} failure(s)",
                self.cleanup_failures.len()
            ),
            "cleanup" => write!(
                formatter,
                "graph provider `{}` cleanup failed at {}",
                self.provider, self.source
            ),
            _ => write!(
                formatter,
                "graph provider `{}` failed during {:?} at {}",
                self.provider, self.phase, self.source
            ),
        }
    }
}

impl Error for ProviderFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let Some(original) = self.original.as_deref() {
            Some(original as &(dyn Error + 'static))
        } else {
            Some(self.error.as_ref() as &(dyn Error + 'static))
        }
    }
}

#[derive(Debug)]
struct ProviderFailureSummary(&'static str);

impl fmt::Display for ProviderFailureSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} details are available through ProviderFailure",
            self.0
        )
    }
}

impl Error for ProviderFailureSummary {}
