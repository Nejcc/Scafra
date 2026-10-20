//! Application-scoped scheduled task support for Scafra.
//!
//! Each [`Scheduler`] owns one Tokio task per enabled registration. A task's
//! synchronous callback runs on Tokio's bounded blocking pool. Invocations of
//! one registration never overlap: the first starts after its interval, and
//! the next starts one interval after the previous callback finishes. Time
//! elapsed during a callback is coalesced; no backlog is queued.
//!
//! The scheduler is process-local. Separate scheduler instances have isolated
//! cancellation and metrics, but both execute the same linked registrations.
//! There is no cross-process coordination, leader election, or exactly-once
//! guarantee.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{
    sync::watch,
    task::JoinHandle,
    time::{sleep_until, Instant},
};

/// Largest supported task interval: one year in milliseconds.
const MAX_INTERVAL_MS: u64 = 31_536_000_000;
const MAX_TASK_NAME_BYTES: usize = 64;

/// Configuration for the Scafra scheduler.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SchedulerConfig {
    /// Scheduler is opt-in; no jobs run unless this is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Per-task overrides keyed by the registered task name.
    #[serde(default)]
    pub tasks: BTreeMap<String, ScheduledTaskConfig>,
}

impl SchedulerConfig {
    /// Checks configured task names and interval values against linked
    /// registrations. Diagnostics intentionally omit configured names and
    /// values.
    pub fn validate(&self) -> Result<(), SchedulerConfigError> {
        let mut names = BTreeSet::new();
        for registration in registered_tasks() {
            if !valid_task_name(registration.name) {
                return Err(SchedulerConfigError::InvalidRegistration);
            }
            if !names.insert(registration.name) {
                return Err(SchedulerConfigError::DuplicateRegistration);
            }
            validate_interval(registration.default_interval_ms)?;
        }

        for (name, task) in &self.tasks {
            if !names.contains(name.as_str()) {
                return Err(SchedulerConfigError::UnknownTask);
            }
            if let Some(interval_ms) = task.interval_ms {
                validate_interval(interval_ms)?;
            }
        }

        Ok(())
    }
}

/// A safe scheduler configuration validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SchedulerConfigError {
    /// A configured task name has no linked registration.
    #[error("scheduler configuration names an unknown task")]
    UnknownTask,
    /// A configured or registered interval is zero.
    #[error("scheduler intervals must be greater than zero milliseconds")]
    ZeroInterval,
    /// An interval exceeds the supported one-year maximum.
    #[error("scheduler intervals must not exceed one year")]
    IntervalTooLarge,
    /// A registration name is empty, too long, or contains unsupported bytes.
    #[error("scheduler task registration has an invalid name")]
    InvalidRegistration,
    /// Two linked task registrations use the same name.
    #[error("scheduler task registrations must have unique names")]
    DuplicateRegistration,
}

/// Configuration for one scheduled task.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScheduledTaskConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Interval in milliseconds. A missing value uses the task's registration
    /// default. Values must be between 1 ms and one year, inclusive.
    #[serde(default)]
    pub interval_ms: Option<u64>,
}

impl Default for ScheduledTaskConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_ms: None,
        }
    }
}

fn default_true() -> bool {
    true
}

fn validate_interval(interval_ms: u64) -> Result<(), SchedulerConfigError> {
    if interval_ms == 0 {
        Err(SchedulerConfigError::ZeroInterval)
    } else if interval_ms > MAX_INTERVAL_MS {
        Err(SchedulerConfigError::IntervalTooLarge)
    } else {
        Ok(())
    }
}

fn valid_task_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_TASK_NAME_BYTES
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn registered_tasks() -> Vec<&'static ScheduledTaskRegistration> {
    let mut registrations = inventory::iter::<ScheduledTaskRegistration>
        .into_iter()
        .collect::<Vec<_>>();
    registrations.sort_by_key(|registration| (registration.name, registration.default_interval_ms));
    registrations
}

fn safe_task_name(name: &str) -> String {
    name.bytes()
        .take(MAX_TASK_NAME_BYTES)
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.') {
                char::from(byte)
            } else {
                '?'
            }
        })
        .collect()
}

/// Link-time registration emitted for scheduled jobs.
pub struct ScheduledTaskRegistration {
    pub name: &'static str,
    pub default_interval_ms: u64,
    pub task: fn(),
}

inventory::collect!(ScheduledTaskRegistration);
pub use inventory;

/// Registers a synchronous scheduled callback.
///
/// The callback runs on Tokio's blocking pool, never on an async worker. It
/// should still return in a bounded amount of time: Rust cannot interrupt a
/// synchronous callback that has started. Prefer asynchronous I/O only after
/// exposing an async task API in a future compatible extension.
#[macro_export]
macro_rules! register_scheduled_task {
    ($name:literal, $interval_ms:expr, $task:path) => {
        $crate::inventory::submit! {
            $crate::ScheduledTaskRegistration {
                name: $name,
                default_interval_ms: $interval_ms,
                task: $task,
            }
        }
    };
}

/// Error returned when scheduler startup cannot be completed.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// Scheduler configuration does not match the linked registrations.
    #[error(transparent)]
    Configuration(#[from] SchedulerConfigError),
    /// Starting enabled tasks requires a current Tokio runtime.
    #[error("starting scheduled tasks requires a Tokio runtime")]
    RuntimeUnavailable,
}

#[derive(Debug, Default)]
struct MetricsInner {
    runs_started: AtomicU64,
    runs_finished: AtomicU64,
    callback_panics: AtomicU64,
    active_callbacks: AtomicUsize,
}

/// A cloneable, scheduler-local metrics handle.
#[derive(Debug, Clone, Default)]
pub struct SchedulerMetrics {
    inner: Arc<MetricsInner>,
}

impl SchedulerMetrics {
    /// Returns a point-in-time copy of the scheduler's execution counters.
    pub fn snapshot(&self) -> SchedulerMetricsSnapshot {
        SchedulerMetricsSnapshot {
            runs_started: self.inner.runs_started.load(Ordering::Relaxed),
            runs_finished: self.inner.runs_finished.load(Ordering::Relaxed),
            callback_panics: self.inner.callback_panics.load(Ordering::Relaxed),
            active_callbacks: self.inner.active_callbacks.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time scheduler execution counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SchedulerMetricsSnapshot {
    /// Callback invocations submitted to the blocking pool.
    pub runs_started: u64,
    /// Callback invocations that returned or panicked.
    pub runs_finished: u64,
    /// Callback panics observed by the scheduler.
    pub callback_panics: u64,
    /// Submitted callbacks that have not returned yet.
    pub active_callbacks: usize,
}

struct ActiveCallback(Arc<MetricsInner>);

impl ActiveCallback {
    fn enter(metrics: Arc<MetricsInner>) -> Self {
        metrics.active_callbacks.fetch_add(1, Ordering::Relaxed);
        Self(metrics)
    }
}

impl Drop for ActiveCallback {
    fn drop(&mut self) {
        self.0.active_callbacks.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Owns the scheduled task loops for one application instance.
///
/// Instances have independent cancellation and metrics state. `stop()` and
/// dropping this value request cancellation and return without waiting.
/// Sleeping loops wake and exit; an active synchronous callback cannot be
/// interrupted, so its loop exits after that callback returns. Use
/// [`Scheduler::shutdown`] when the caller must wait for all loops and active
/// callbacks to finish.
pub struct Scheduler {
    tasks: Option<Vec<JoinHandle<()>>>,
    cancellation: watch::Sender<bool>,
    metrics: SchedulerMetrics,
}

/// A cloneable signal that asks one scheduler instance to stop starting work.
/// The owner should still call [`Scheduler::shutdown`] to join its task loops.
#[derive(Clone)]
pub struct SchedulerShutdownHandle {
    cancellation: watch::Sender<bool>,
}

impl fmt::Debug for SchedulerShutdownHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("SchedulerShutdownHandle").finish()
    }
}

impl SchedulerShutdownHandle {
    /// Requests cooperative cancellation of this scheduler's loops.
    pub fn request_shutdown(&self) {
        self.cancellation.send_replace(true);
    }
}

impl fmt::Debug for Scheduler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Scheduler")
            .field("task_count", &self.tasks.as_ref().map_or(0, Vec::len))
            .field("metrics", &self.metrics.snapshot())
            .finish_non_exhaustive()
    }
}

impl Scheduler {
    /// Starts enabled registrations.
    ///
    /// This compatibility wrapper logs a safe startup error and returns an
    /// inert scheduler if validation fails or no Tokio runtime is active. New
    /// code should use [`Scheduler::try_start`] to handle startup errors.
    pub fn start(config: &SchedulerConfig) -> Self {
        match Self::try_start(config) {
            Ok(scheduler) => scheduler,
            Err(error) => {
                tracing::error!(reason = %error, "scheduler startup failed");
                Self::empty()
            }
        }
    }

    /// Validates configuration and starts the enabled registrations.
    ///
    /// No tasks is a valid no-op, including when the scheduler is enabled. The
    /// first callback starts after its configured interval. Subsequent
    /// callbacks start one interval after the prior callback finishes.
    pub fn try_start(config: &SchedulerConfig) -> Result<Self, SchedulerError> {
        config.validate()?;

        let mut enabled = registered_tasks()
            .into_iter()
            .filter_map(|registration| {
                let task_config = config.tasks.get(registration.name);
                if !config.enabled || task_config.is_some_and(|task| !task.enabled) {
                    return None;
                }
                let interval_ms = task_config
                    .and_then(|task| task.interval_ms)
                    .unwrap_or(registration.default_interval_ms);
                Some((
                    safe_task_name(registration.name),
                    Duration::from_millis(interval_ms),
                    registration.task,
                ))
            })
            .collect::<Vec<_>>();
        enabled.sort_by(|left, right| left.0.cmp(&right.0));

        if enabled.is_empty() {
            return Ok(Self::empty());
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| SchedulerError::RuntimeUnavailable)?;
        let metrics = SchedulerMetrics::default();
        let (cancellation, _) = watch::channel(false);
        let started_at = Instant::now();
        let tasks = enabled
            .into_iter()
            .map(|(name, interval, callback)| {
                let receiver = cancellation.subscribe();
                let inner_metrics = Arc::clone(&metrics.inner);
                runtime.spawn(run_task(
                    name,
                    interval,
                    callback,
                    receiver,
                    inner_metrics,
                    started_at + interval,
                ))
            })
            .collect();

        Ok(Self {
            tasks: Some(tasks),
            cancellation,
            metrics,
        })
    }

    fn empty() -> Self {
        let (cancellation, _) = watch::channel(false);
        Self {
            tasks: Some(Vec::new()),
            cancellation,
            metrics: SchedulerMetrics::default(),
        }
    }

    /// Returns this scheduler's isolated metrics handle.
    pub fn metrics(&self) -> SchedulerMetrics {
        self.metrics.clone()
    }

    /// Returns a handle that can request cancellation before the owner awaits
    /// [`Scheduler::shutdown`]. This is useful while an HTTP server drains.
    pub fn shutdown_handle(&self) -> SchedulerShutdownHandle {
        SchedulerShutdownHandle {
            cancellation: self.cancellation.clone(),
        }
    }

    /// Requests cancellation without waiting for task loops or active
    /// callbacks to finish. A synchronous callback already running on the
    /// blocking pool continues until it returns.
    pub fn stop(self) {
        self.request_cancellation();
    }

    /// Requests cancellation and waits for every owned task loop and callback
    /// to finish. A synchronous callback that never returns makes this future
    /// wait indefinitely because Rust cannot safely interrupt it.
    pub async fn shutdown(mut self) {
        self.request_cancellation();
        let Some(tasks) = self.tasks.take() else {
            return;
        };
        for task in tasks {
            if task.await.is_err() {
                tracing::warn!(
                    outcome = "task_join_failed",
                    "scheduler task stopped unexpectedly"
                );
            }
        }
    }

    fn request_cancellation(&self) {
        self.cancellation.send_replace(true);
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        self.request_cancellation();
        // Dropping a JoinHandle detaches it. The cancellation watch wakes any
        // sleeper, and active callbacks finish before their loops observe it.
        // `shutdown()` is the API that joins the handles.
    }
}

async fn run_task(
    name: String,
    interval: Duration,
    callback: fn(),
    mut cancellation: watch::Receiver<bool>,
    metrics: Arc<MetricsInner>,
    first_run: Instant,
) {
    let mut next_run = first_run;
    loop {
        if *cancellation.borrow() {
            return;
        }
        tokio::select! {
            biased;
            changed = cancellation.changed() => {
                if changed.is_err() || *cancellation.borrow() {
                    return;
                }
            }
            _ = sleep_until(next_run) => {}
        }
        if *cancellation.borrow() {
            return;
        }

        metrics.runs_started.fetch_add(1, Ordering::Relaxed);
        let active = ActiveCallback::enter(Arc::clone(&metrics));
        let join = tokio::task::spawn_blocking(move || {
            let _active = active;
            callback();
        });
        let outcome = join.await;
        metrics.runs_finished.fetch_add(1, Ordering::Relaxed);
        match outcome {
            Ok(()) => tracing::debug!(
                task = %name,
                outcome = "completed",
                "scheduled callback completed"
            ),
            Err(error) if error.is_panic() => {
                metrics.callback_panics.fetch_add(1, Ordering::Relaxed);
                // Do not format JoinError: its panic payload may contain
                // secrets or arbitrary application state.
                tracing::warn!(
                    task = %name,
                    outcome = "panicked",
                    "scheduled callback panicked; the next scheduled invocation remains enabled"
                );
            }
            Err(_) => tracing::warn!(
                task = %name,
                outcome = "cancelled",
                "scheduled callback did not complete"
            ),
        }

        // Fixed delay: no ticks accumulate while the callback runs. Any time
        // elapsed during the callback is coalesced into this next interval.
        next_run = Instant::now() + interval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_scheduler_with_no_linked_tasks_is_a_valid_noop() {
        let scheduler = Scheduler::try_start(&SchedulerConfig {
            enabled: true,
            tasks: Default::default(),
        })
        .expect("no registered tasks need no runtime and are valid");
        assert_eq!(
            scheduler.metrics().snapshot(),
            SchedulerMetricsSnapshot::default()
        );
    }
}
