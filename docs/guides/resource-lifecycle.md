# Asynchronous resource lifecycle

Use async bean providers for resources that must finish I/O setup before the
application serves traffic. The generated graph awaits each provider in its
existing compile-time dependency order and retains its typed output for
shutdown. It does not create a runtime service map.

## Provider and ownership contract

An async provider returns an owned, concrete output. Fallible initialization
uses `Result<T, E>`:

```rust
use std::{convert::Infallible, sync::Arc};
use scafra::{bean, AsyncResource, ShutdownPolicy, ShutdownReason};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

pub struct DatabasePool {
    pool: SqlitePool,
}

impl AsyncResource for DatabasePool {
    type ShutdownError = Infallible;

    async fn shutdown(
        &self,
        _reason: ShutdownReason,
        _policy: ShutdownPolicy,
    ) -> Result<(), Self::ShutdownError> {
        self.pool.close().await;
        Ok(())
    }
}

#[bean]
pub async fn database_pool() -> Result<DatabasePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    Ok(DatabasePool { pool })
}
```

The example uses a real SQLx pool backed by SQLite in memory. A production
application can keep the same application-owned newtype around its chosen
pool or client, so it can implement Scafra traits without violating Rust's
orphan rules. Scafra does not require a database dependency.

Providers are called once per graph composition. Independent providers with
the same dependency depth are ordered by output type name, making startup and
rollback deterministic. Providers are initialized sequentially. A consumer of
an async resource must declare `Arc<T>`:

```rust
#[service]
pub struct UserRepository {
    pool: Arc<DatabasePool>,
}
```

The generated graph owns one `Arc` for lifecycle cleanup and passes cloned
handles to its typed consumers. This is required even for a single consumer:
it keeps the resource alive until shutdown. A missing `AsyncResource`
implementation or an owned (non-`Arc`) async dependency fails at compile time.

The async output does not receive a generated `Default` implementation. Runtime
configuration and fallible I/O remain explicit arguments to the provider.

## Startup, readiness, and rollback

The standard `#[scafra::main]` macro uses `compose_async()`. The runner waits
for all async providers, builds and validates routes, starts lifecycle hooks
and the scheduler, and binds the HTTP listener before serving requests. A
required provider error returns from startup before the listener is bound, so
the application cannot become ready with a missing required resource.

If a later provider fails, the generated graph awaits shutdown for every
earlier async provider in reverse dependency order. It attempts all cleanup
operations even if one fails. `GraphError::original_provider_failure()` keeps
the original typed error identifiable; `GraphError::cleanup_failures()` keeps
all rollback errors. The source chain remains available through
`std::error::Error::source`.

The provider that returns `Err` still owns any partial work that it created
before returning. It must clean that work itself or return an RAII guard that
does so on drop. Scafra can only roll back resources that a provider
successfully returned. If the application task itself is cancelled while a
provider future is in flight, Rust drops that future; Scafra cannot safely
force an arbitrary network operation to stop. Providers should avoid
detaching tasks and should make their in-flight operations cancellation-safe.

## Shutdown order and deadlines

On shutdown, Scafra requests scheduler and resource cancellation before HTTP
request draining begins. The listener stops accepting new connections while
Axum drains active requests according to `ShutdownPolicy`. After the server
finishes draining, Scafra waits for scheduler callbacks, runs application
shutdown hooks in reverse hook order, and then awaits async resource teardown
in reverse dependency order.

Implement `AsyncResource::request_shutdown` for a prompt, idempotent signal
that prevents new background work. Implement `shutdown` to join work owned by
the resource and release the resource. The supplied `ShutdownReason` and
`ShutdownPolicy` let the resource apply its own grace period. The generated
resource set keeps cleanup running after an individual shutdown error and
returns all failures.

When `ShutdownPolicy::force_after_grace()` is true, `grace_period` bounds HTTP
request draining; with it false, the server waits for draining without a
deadline. The runner does not use that HTTP deadline to drop arbitrary
scheduler or resource shutdown futures: doing so can detach owned work or leave
external operations in an unknown state. A scheduler callback that never
returns can therefore delay shutdown indefinitely. Lifecycle hooks are
synchronous too, so a blocking hook can delay resource teardown indefinitely.
A resource with a Tokio task can use the supplied policy and abort that task
after its grace period if its work is safe to abort. Aborting a task cannot
undo an external side effect or interrupt arbitrary blocking code. A resource
that ignores its policy can also delay shutdown indefinitely.

The router is dropped after HTTP draining and before the generated resource
set is shut down. This means graph consumers stop receiving requests before
their resource is torn down. Custom runners must preserve that order and keep
the returned resource set alive for the full serving lifetime.

## Custom runner and testing

The standard macro owns this flow. A custom application runner can use the
additive `run_with_resources` entry point. Its builder receives an owned
configuration value and returns both the router and the resource set. Generated
graphs expose `build_router_and_resources`; it returns the route result and the
resource owner separately so route validation failures can still clean up
initialized resources.

When a custom builder has already composed resources and then encounters an
error, it must call `AsyncResourceSet::request_shutdown` and await
`AsyncResourceSet::shutdown` before returning the error. The generated
`#[scafra::main]` runner does this automatically.

The runnable example uses the real generated graph, a SQLx SQLite pool backed
by `sqlite::memory:`, a dependent client, a cooperative background task,
failure injection,
and explicit teardown:

```bash
cargo check --manifest-path examples/async-resource/Cargo.toml
cargo test --manifest-path examples/async-resource/Cargo.toml
SCAFRA_DEV_RELOAD=false ASYNC_RESOURCE_EXAMPLE_FAIL_RESOURCE=worker cargo run --manifest-path examples/async-resource/Cargo.toml
```

The last command exits during startup after earlier resources are rolled back.
Set `ASYNC_RESOURCE_EXAMPLE_FAIL_CLEANUP=client` to demonstrate cleanup-error
reporting. `ASYNC_RESOURCE_EXAMPLE_IGNORE_WORKER_CANCELLATION=true` exercises
the example worker's declared timeout and abort behavior.

## Compatibility and issue #74

Synchronous `compose()` and `compose_with()` remain available for graphs with
only synchronous providers. Existing `run_with_router`, lifecycle hooks,
synchronous providers, services, and controllers keep their signatures. The
standard generated main path now uses `compose_async()` and retains resources
through the server lifetime.

The bean macro still has the broader external-output and generated-`Default`
design question tracked by [issue #74](https://github.com/Scafra/Scafra/issues/74).
Use an application-owned newtype for external pool/client types. This issue
does not change the synchronous bean macro's existing `Default` behavior.
