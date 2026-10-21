# Typed dependency graph

Scafra generates a typed dependency graph from application source during
compilation. The standard `#[scafra::main]` path composes services, beans, and
controllers through their constructors before the server starts.

The graph can also be generated explicitly from an application-owned build
script with `scafra_build::discover_graph`.

## Bean provider contract

`#[bean]` marks a public, non-generic function as the constructor for one graph
output. The function must return an owned, concrete application type, either
directly or as `Result<T, E>`. Async functions may also use the fallible form.
Provider arguments are dependencies: Scafra resolves them from the generated
typed graph and passes the already constructed values to the function.

```rust
use scafra::{bean, service};

pub struct RuntimeConfig {
    user_agent: String,
}

pub struct Credentials(String);

// This is an application-owned wrapper around a third-party HTTP client.
pub struct ApiClient {
    _client: reqwest::Client,
    _credentials: Credentials,
}

#[bean]
pub fn runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        user_agent: "catalog-service/1.0".to_owned(),
    }
}

#[bean]
pub fn credentials() -> Credentials {
    Credentials("loaded from application configuration".to_owned())
}

#[bean]
pub fn api_client(
    config: RuntimeConfig,
    credentials: Credentials,
) -> Result<ApiClient, reqwest::Error> {
    let client = reqwest::Client::builder()
        .user_agent(config.user_agent)
        .build()?;
    Ok(ApiClient {
        _client: client,
        _credentials: credentials,
    })
}

#[service]
pub struct ApiService {
    client: ApiClient,
}
```

`RuntimeConfig` and `Credentials` need no `Default` implementation. Scafra
calls each provider once for each graph composition, in deterministic
dependency order, and passes those values directly. It does not create
provider arguments with
`Default::default()` or invoke providers again to synthesize a default value.

Provider outputs must be public structs declared in application source. A raw
third-party output such as `reqwest::Client` is unsupported: Scafra validates
and names outputs from the application's source declarations. Rust's orphan
rules also prevent an application macro from implementing Scafra's `Bean` or
`AsyncResource` traits for a third-party type. Wrap the resource in a local
newtype and return that newtype instead. For an async provider, implement
`AsyncResource` on the wrapper so it can own shutdown behavior. The graph
itself does not require `Bean`; that marker remains available for applications
that want a local trait bound.

The macro no longer generates `Default` for provider outputs. If standalone
code needs a default value, add an explicit `#[derive(Default)]` or a manual
implementation that reflects the application's actual default semantics.
Existing manual `Default` implementations continue to work without conflicts.

### Migration from generated implementations

If code relied on the generated `Default`, add an explicit derive or
implementation. If code used `T: Bean` to identify provider outputs, implement
`Bean` on application-owned types that need that bound. Graph composition does
not use `Bean`, so removing that bound from application code is also valid.

## When to use it

Annotate services and providers with `#[service]` and `#[bean]`, then list
dependency types as constructor fields or provider arguments. The graph checks
missing providers and cycles during compilation. Synchronous fallible providers
and asynchronous providers run during startup. Shared dependencies use
`Arc<T>` on every consumer; the graph constructs one value and clones its
handle for each consumer.

Async providers use an `async fn` and return an owned concrete resource,
normally as `Result<T, E>`. The output type implements `AsyncResource`, and
every consumer declares `Arc<T>` so the graph can retain the resource for
shutdown. Initialization uses `compose_async()` and is sequential in graph
order. See the [resource lifecycle guide](resource-lifecycle.md) for rollback,
shutdown, and an integration example.

Sync or async provider arguments are graph dependencies, not values to be
defaulted. A dependency type must have one provider in the graph. Unsupported
provider outputs receive a source-aware build diagnostic that recommends a
public application-owned wrapper. Qualified or generic output paths are not
accepted by the generated graph; imports for a local wrapper may remain
unqualified in the provider signature.

Services and controllers are constructed with their generated `new(...)`
constructors, so graph injection does not require `Default`. For compatibility
with standalone `build_router()`, use `#[routes(default)]` on controllers that
implement `Default`. Graph-composed controllers use `#[routes]` and are
registered with their constructed instances. When composing asynchronously,
use `Graph::build_router_and_resources` and pass the returned resource set to
`run_with_resources`; the legacy `build_router` method drops that set and is
intended for graphs without async providers.

## Important boundaries

- graph declarations are discovered at compile time from `src/main/`;
- dependencies are represented by Rust types, not string names;
- the graph does not provide a runtime service locator;
- generated controllers receive constructed dependencies before route
  registration;
- `#[scafra::main]` owns standard startup and graceful shutdown.

## Graph-scoped health checks

Application dependency checks belong to a `scafra::HealthRegistry`. Register
checks before building the router, capture an owned handle to the relevant
service (usually `Arc<T>`), then pass the registry to
`Graph::build_router_with_health_registry`. Each graph gets a separate
registry, and the generated router retains it for the router's serving
lifetime. The generated graph does not discover health checks from service
types automatically.

The standard `#[scafra::main]` builder remains compatible with
`register_health_check!`, whose inventory registrations are process-wide and
cannot inspect a particular graph. Applications that need graph-owned service
state should use a custom `run_with_router` builder and the generated graph
method. Duplicate or empty registry names are rejected. An unknown configured
check is rejected when the graph-scoped router is built; an unknown selection
on the legacy router reports `DOWN`.

See the implementation and tests in `crates/scafra-build` for the current API
surface and supported graph shapes.
