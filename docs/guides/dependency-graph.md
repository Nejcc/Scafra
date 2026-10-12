# Typed dependency graph

Scafra generates a typed dependency graph from application source during
compilation. The standard `#[scafra::main]` path composes services, beans, and
controllers through their constructors before the server starts.

The graph can also be generated explicitly from an application-owned build
script with `scafra_build::discover_graph`.

## When to use it

Annotate services and providers with `#[service]` and `#[bean]`, then list
dependency types as constructor fields or provider arguments. The graph checks
missing providers and cycles during compilation and constructs fallible bean
providers during startup. Shared dependencies use `Arc<T>` on every consumer;
the graph constructs one value and clones its handle for each consumer.

Services and controllers are constructed with their generated `new(...)`
constructors, so graph injection does not require `Default`. For compatibility
with standalone `build_router()`, use `#[routes(default)]` on controllers that
implement `Default`. Graph-composed controllers use `#[routes]` and are
registered with their constructed instances.

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
