# Scheduled tasks

Scafra's scheduler runs link-time task registrations in one application
instance. Register a synchronous callback with the existing macro:

~~~rust
scafra::register_scheduled_task!("cleanup", 60_000, CleanupService::cleanup);
~~~

Set `scheduler.enabled: true` to run registered tasks. An individual task is
enabled by default when the scheduler is enabled; set
`scheduler.tasks.cleanup.enabled: false` to disable it. An omitted
`interval_ms` uses the registration's default. Intervals must be at least
1 millisecond and at most one year. Zero, larger intervals, duplicate or
invalid registrations, and configured names without a linked registration
fail configuration validation. Diagnostics identify the configuration field
and reason without printing configured task names or interval values.

An enabled scheduler with no registered tasks is a valid no-op. A disabled
scheduler starts no task loops. `Scheduler::try_start` returns a startup error
if configuration is invalid or enabled work is started without a current Tokio
runtime. `Scheduler::start` remains as a compatibility wrapper; it logs a
safe startup error and returns an inert scheduler. New code should use
`try_start` so startup failures can be handled.

`StartupError` now includes a `Scheduler` variant for failures from the
standard runner. Downstream exhaustive matches should handle that variant;
the existing scheduler constructor and stop signatures remain available.

## Timing and overlap

The first invocation begins one configured interval after scheduler startup.
Each registration runs serially within a scheduler instance. Its next
invocation begins one interval after the previous callback returns. If a
callback takes longer than its interval, no extra invocation is queued and
there is no catch-up run; the schedule continues from the callback's
completion. This fixed-delay policy bounds execution to one active invocation
per registration and does not create an execution queue. The current public
configuration does not offer multiple overlap policies.

Callbacks are submitted to Tokio's blocking pool, so their synchronous work
does not occupy an async worker. Keep callbacks bounded: Tokio's blocking pool
has a finite thread limit, and a callback that never returns can consume a
blocking thread indefinitely. Rust cannot forcibly interrupt synchronous
code. The scheduler does not provide a separate async callback API today.

## Failure and observability

The callback type returns `()`, so there is no fallible-task result to report.
A callback panic is isolated to that invocation. The scheduler records a
panic, emits a structured warning containing the bounded task name and
outcome, and permits the next normal invocation. It does not immediately
retry or restart the task loop. Scheduler diagnostics do not format panic
payloads or callback values. Rust's process panic hook still observes a panic
and may emit its own output according to application configuration.

Clone `Scheduler::metrics()` before shutdown to inspect per-instance counters:
started and finished invocations, callback panics, and currently submitted
callbacks. The scheduler also emits a structured debug event for a completed
callback and a warning for a panic or unexpected task cancellation. These
counters are process-local observations, not a distributed metrics backend.

## Shutdown and ownership

Each `Scheduler` owns its task loops, cancellation signal, and metrics. Two
instances in one process have isolated shutdown and metrics state, but both
will run the same linked registrations.

`stop(self)` requests cancellation and returns without waiting. Dropping a
scheduler has the same cancellation-request behavior. Sleeping loops wake and
exit, and no further callback is scheduled. A callback already running on the
blocking pool cannot be interrupted; its current invocation continues, then
its loop observes cancellation. Use `shutdown(self).await` when shutdown must
wait for all scheduler loops and active callbacks. `shutdown` has no timeout:
it waits indefinitely if a synchronous callback never returns.

Scafra's standard runner starts the scheduler after application lifecycle
startup. When shutdown is requested, it signals the scheduler before HTTP
request draining begins. After the server drains, the runner waits for active
scheduler callbacks, runs application shutdown hooks, and then shuts down
async resources. A scheduler startup error triggers application and resource
cleanup.

The request signal does not interrupt an active synchronous callback. The
runner still owns its join handle and waits for that callback before resource
teardown, so a callback that never returns can delay shutdown indefinitely.
`ShutdownPolicy` bounds HTTP request draining; a resource that needs a bounded
shutdown must apply the supplied policy in its own `AsyncResource` teardown.

Scheduler ownership is local to a process. Multiple processes or replicas
each execute their own callbacks. Use an external lease, leader election, or
another coordination mechanism if deployment-wide single execution is
required; Scafra does not provide distributed locking or exactly-once
execution.
