# Scafra

Scafra is an open-source, batteries-included web application framework for Rust.
It gives Rust applications a clear structure for composition, configuration,
startup, routing, logging, security, and operational endpoints while keeping
the underlying Rust ecosystem visible and accessible.

Scafra is designed for teams that want the productive defaults commonly found
in larger application frameworks without giving up Rust's compile-time safety,
performance, or control. It builds on familiar technologies such as Axum,
Tokio, Tower, Serde, and `tracing` instead of hiding them behind a proprietary
runtime.

## What is Scafra?

Scafra provides the application infrastructure around a Rust web service. Its
facade crate and procedural macros let developers define typed services,
controllers, routes, configuration, lifecycle hooks, and scheduled tasks in a
consistent application model. The CLI can generate a ready-to-run project so
new applications start with useful conventions instead of an empty directory.

The current MVP focuses on one complete path and a first shape-aware project
generator:

~~~text
typed components -> generated routes -> Axum router -> Tokio server
~~~

The starter generator currently supports web applications, JSON APIs,
services, and modular monoliths. The starters use the same Scafra programming
model; the selected shape changes the sample code and layout, not the runtime
architecture.

## Why use Scafra?

- **Compile-time safety.** Services, dependencies, routes, and configuration
  are represented with ordinary Rust types and compiler-checked code. Scafra
  does not depend on runtime reflection, string-key lookups, or a global service
  locator.
- **Fast project startup.** `scafra new` creates a working application with a
  Cargo manifest, configuration defaults, source layout, and a representative
  example.
- **Batteries included.** The framework provides conventions for routing,
  configuration profiles, structured logging, graceful shutdown, health and
  readiness endpoints, metrics, security options, and scheduled tasks.
- **Familiar Rust foundations.** Scafra extends Axum, Tokio, Tower, Serde, and
  `tracing`; developers can use their existing knowledge and add direct
  dependencies when they need lower-level control.
- **Clear application structure.** Generated applications separate controllers,
  services, repositories, configuration, models, and errors. This makes the
  codebase easier to navigate as the application grows.
- **Operational by default, secure by design.** The default listener is local,
  request bodies are limited, request bodies are not logged, and operational
  features such as actuator endpoints and authentication are opt-in and
  configurable.
- **A gradual path from convention to control.** Teams can use the generated
  application model, customize the router and components, or replace the
  standard runner with an application-owned Axum/Tokio entry point when a more
  specialized server boundary is needed.

Scafra is currently an MVP. The implemented application shapes are web
applications, JSON APIs, services, and modular monoliths.

## Why Rust developers choose Scafra

Scafra is intended to feel like Rust, not like a foreign runtime dropped on top
of Rust. The framework favors explicit types, normal Cargo projects, compiler
errors, and libraries from the standard Rust web ecosystem.

- **No hidden runtime magic.** Component discovery and route registration are
  generated at build time. There is no runtime filesystem scan, reflection
  layer, or string-based dependency container to debug in production.
- **Generated code stays inspectable.** Scafra's CLI writes ordinary Rust source
  files and a normal Cargo project. When something goes wrong, developers can
  read the generated project, follow the compiler diagnostics, and take
  ownership of the code.
- **Use the ecosystem you already know.** Scafra exposes Axum, Tower, Serde,
  Tokio, and `tracing` at the application boundary. Existing middleware,
  extractors, serializers, test tools, and libraries remain useful.
- **Abstractions without giving up control.** The standard runner provides
  sensible application startup and shutdown, while `build_router()` and the
  lower-level re-exports make it possible to customize the HTTP boundary.
- **Adopt it incrementally.** Start with one service or generated application,
  keep direct Rust code where it is clearer, and introduce Scafra conventions
  only where they reduce repetition.
- **Designed for performance-sensitive services.** Scafra builds on async Rust
  and Axum rather than introducing a separate execution model. The framework's
  goal is to organize application code without turning the hot path into a
  dynamic object graph.

## Why contribute to Scafra?

Scafra is also a place to improve the Rust web development experience itself.
Contributors can work on focused crates instead of one large runtime: macros,
configuration, lifecycle management, routing, security, scheduling, the CLI,
or the project generator can evolve independently behind clear boundaries.

The project is a good fit for Rust developers who want to:

- shape practical conventions for Rust applications;
- improve compile-time APIs, diagnostics, and generated-code ergonomics;
- build reusable tooling for Axum and the wider async Rust ecosystem;
- contribute examples, tests, documentation, or starter templates; and
- help decide which framework features belong in Scafra — and which should stay
  in existing ecosystem crates.

The current MVP keeps the scope deliberately understandable. New contributors
can run the workspace checks, inspect a focused crate, add a test or example,
and discuss a concrete improvement without needing to learn a large runtime
first. See the [documentation index](docs/README.md) and the [architecture
guide](docs/architecture.md) before opening a design-heavy change.

## Quick start from this workspace

~~~bash
cargo run -p scafra-cli -- new hello-world
cd hello-world
cargo run
# In another terminal:
curl http://127.0.0.1:8080/hello/Alice
# Hello, Alice!
~~~

The same application shape is available to users through the facade crate:

~~~rust
use scafra::prelude::*;

#[service]
struct GreetingService;

impl GreetingService {
    fn greet(&self, name: &str) -> String {
        format!("Hello, {name}!")
    }
}

#[controller("/api")]
struct GreetingController {
    service: GreetingService,
}

#[routes]
impl GreetingController {
    #[get("/hello/{name}")]
    async fn hello(&self, name: Path<String>) -> String {
        self.service.greet(&name)
    }
}

#[scafra::main]
async fn main() {}
~~~

'#[service]' generates a typed constructor. With '#[scafra::main]', Scafra
composes services, beans, and controllers through the typed graph before
starting the server. Components do not need 'Default' for constructor
injection; add '#[derive(Default)]' when direct default construction is needed.
'#[bean]' turns a concrete provider function into a typed provider, and
'#[routes]' generates ordinary Axum handler adapters. Applications can still
use 'scafra_build::discover_graph' for an application-owned build pipeline;
see the [typed graph guide](docs/guides/dependency-graph.md). Runtime
reflection and a global service locator are not used.

## Create an application

From this checkout, create a starter with `scafra new`:

~~~bash
cargo run -p scafra-cli -- new storefront
cargo run -p scafra-cli -- new billing-api --kind api
cargo run -p scafra-cli -- new catalog --kind service
cargo run -p scafra-cli -- new admin --kind web
cargo run -p scafra-cli -- new backoffice --kind monolith
~~~

Omitting `--kind` defaults to `web`. The canonical values are `web`, `api`,
`service`, and `monolith`; `microservice` is accepted as an alias for
`service`.

| Kind | Representative route | Sample response |
| --- | --- | --- |
| `web` | `GET /hello/Alice` | `Hello, Alice!` |
| `api` | `GET /api/greetings/Alice` | JSON `{"message":"Hello, Alice!"}` |
| `service` | `GET /health` | `ok` |
| `monolith` | `GET /catalog/items` | JSON catalog items |

The API response is `application/json` with body
`{"message":"Hello, Alice!"}`. The monolith response is also
`application/json`, with the sample body
`[{"id":1,"name":"Notebook"},{"id":2,"name":"Pen"}]`.

Run a generated project with the standard Rust toolchain:

~~~bash
cd billing-api
cargo check
cargo run
~~~

In another terminal:

~~~bash
curl http://127.0.0.1:8080/api/greetings/Alice
~~~

Scafra supports Spring Boot Actuator-style operational endpoints. They are
opt-in and can be selected individually or all at once:

They can be disabled in `application.yaml`:

~~~yaml
actuator:
  endpoints: [health, live, ready, info]
  security:
    enabled: false
  health:
    checks: [database]
~~~

Use `endpoints: "*"` for all endpoints, or
`actuator.endpoints=health,info` in `application.properties`.

Metrics can be exposed with `metrics`, and actuator endpoints can be protected
with `actuator.security.enabled=true` and a bearer token. For application
dependencies, build a `HealthRegistry` for each graph and move it into the
actuator router before serving requests:

~~~rust,ignore
let store = std::sync::Arc::new(PetStore::new());
let store_for_check = std::sync::Arc::clone(&store);
let mut health = scafra::HealthRegistry::new();
health.register("pet-store", move || {
    let store = std::sync::Arc::clone(&store_for_check);
    async move { store.ping().await.is_ok() }
})?;

let router = scafra::actuator::router_with_health_registry(
    &config.actuator,
    health,
)?;
~~~

The closure owns the service handle for this application. Build separate
registries for separate graphs, even when their checks share a name. The
registry is consumed by router construction and stays fixed for the router's
lifetime; register checks before serving starts. Registration rejects empty or
whitespace-only names and duplicate names within one registry.

Generated dependency graphs also expose
`Graph::build_router_with_health_registry`. Use it from a custom
`run_with_router` startup path when the registry captures graph-owned service
handles. `compose_with` can receive an application-owned `Arc` while a clone is
captured by a health check. The standard `#[scafra::main]` path keeps using the
legacy static registration path for compatibility; graph-scoped registration
is explicit and is not discovered automatically from service types.

`register_health_check!` remains available for simple checks that only inspect
process-wide state. Its inventory registrations are process-global, run for
every router built with a compatibility constructor such as
`router(&ActuatorConfig)` or `build_router_with_actuator`, and cannot access a
particular graph's services. Existing users can migrate by creating a
`HealthRegistry`, registering closures that capture their service handles, and
using `router_with_health_registry` or the generated graph method.

An empty `actuator.health.checks` list selects all checks in that router's
registry; `*` also selects all checks. An empty registry has no dependency
checks and reports `UP`. A configured unknown name is rejected during
graph-scoped router construction. The legacy constructor cannot return a
construction error, so it reports `DOWN` for an unknown selection. Unknown
names are not included in diagnostics. With `*`, any additional configured
names are still validated and must exist.

Checks may perform asynchronous I/O and return `false` when a dependency is
unhealthy. They must not block synchronously while being polled. Each selected
check runs sequentially with a 1000 ms timeout by default; set
`actuator.health.check_timeout_ms` to change the per-check timeout. A check that
returns `false`, unwinds while being created or polled, or times out marks
readiness and aggregate health `DOWN` (HTTP 503). Panic payloads are not included
in actuator responses or Scafra's health-check log messages. With `panic=abort`,
a panic terminates the process. A timeout drops the async future, but work
detached by that future and external side effects cannot be cancelled or
undone. A cancelled probe request also drops its in-flight check future.

To migrate a synchronous check, make it an `async fn` and use asynchronous I/O;
there is no safe way to interrupt an arbitrary blocking Rust function after it
starts. If code constructs `HealthConfig` with a struct literal, add
`..HealthConfig::default()` or set `check_timeout_ms` explicitly.

The readiness aliases `/ready`, `/health/ready`, and
`/actuator/health/readiness` evaluate the configured checks; a selected check
that fails returns `DOWN` with HTTP 503. The liveness aliases `/live`,
`/health/live`, and `/actuator/health/liveness` return `UP` without running
dependency checks, so a dependency failure does not make the process appear
dead. Aggregate health at `/health` and `/actuator/health` uses the same selected
checks as readiness. Actuator authorization is checked before any protected
health check executes. Multiple application graphs in one process are
supported when each uses its own `HealthRegistry`; legacy static checks remain
shared by design.

Scafra's application security is opt-in. It supports a shared bearer token,
HTTP Basic authentication, or JWT resource-server validation. JWT can use a
local HS256 secret for development, or discover RSA signing keys from an OIDC
issuer:

~~~yaml
security:
  enabled: true
  hide_unauthorized: false
  jwt:
    enabled: true
    issuer_uri: https://issuer.example.com
    audiences: [scafra-api]
    required_scopes: [api.read]
~~~

For local development, replace `issuer_uri` with a development-only
`secret`. A direct `jwk_set_uri` can be used when issuer discovery is not
available; production secrets should be supplied through Scafra's environment
configuration. See the [HTTP security guide](docs/security.md) for controller
and route policies, credential claim formats, `401`/`403` behavior, and
migration details.

Application logging is available directly from the prelude. Scafra re-exports
structured `tracing` macros, so services do not need to call the foundation
module explicitly:

~~~rust
use scafra::prelude::*;

#[service]
struct OrderService;

#[logger]
impl OrderService {
    fn create(&self) {
        info!(operation = "create", "creating order");
        warn!("order has no customer");
    }
}
~~~

`#[logger]` can also be placed on a function. It creates a structured span
with the service type and function name; arguments and `self` are excluded by
default so logging does not accidentally require `Debug` or expose values.

The optional `scafra-bootui` dependency uses this configuration. It is disabled
and local-only by default. `bootui.host` must be an IPv4 or IPv6 address
literal; the default is the IPv4 loopback address:

~~~yaml
bootui:
  enabled: true
  host: 127.0.0.1
  port: 8091
  path: /bootui
  local_only: true
~~~

The dashboard is added explicitly by the application:

~~~rust
let router = scafra_bootui::layer(router, &config.bootui);
~~~

`scafra_bootui::layer(router, &config.bootui)` remains available for existing
applications. It loads the rest of the application configuration once when
the router is composed. The checked `try_layer_with_config` API is preferred
when configuration comes from a custom source because it returns policy errors
to the caller. If that compatibility wrapper cannot load application settings,
it warns and uses the default server link settings; use the checked API to
avoid that fallback.

BootUI's `/api/config` response is an allow-listed dashboard snapshot containing
only `server.host` and `server.port`, which the dashboard uses for actuator
links. It does not serialize the application configuration. Security settings
and credentials—including bearer tokens, Basic passwords, JWT/HMAC secrets,
client secrets, and signing keys—are excluded, as are fields not explicitly
added to the snapshot type. The dashboard still displays the returned server
address and port; applications that previously consumed the full configuration
JSON from this endpoint must migrate to this two-field shape.

With `local_only: true`, BootUI accepts only loopback bind addresses, including
`127.0.0.1` and `::1`. A non-loopback address such as `0.0.0.0`, `::`, or a
private/public interface causes standalone startup to fail before a listener
is created, and the embedded router API returns a policy error. Embedded
routes also reject non-loopback peers; custom Axum servers must provide
`ConnectInfo<SocketAddr>` for these routes to work. Scafra's server adapter
provides it automatically.

With `local_only: false`, BootUI requires `security.enabled: true` and valid
Scafra credentials. BootUI applies authentication to every dashboard route,
even if `security.permit_all` names one of those routes. Without valid
authentication, standalone startup fails before binding and the checked
embedded API returns an error. Scafra does not terminate TLS: bind to a trusted
interface, restrict direct access with a firewall, and put the dashboard behind
a trusted HTTPS terminator before allowing network access. Do not expose
BootUI publicly without those protections.

When using Scafra's standard runner, add `use scafra_bootui as _;` to the
application entrypoint so the optional dependency is linked and appears in the
startup diagnostics.

The active profile can be selected from either supported configuration format:

~~~yaml
scafra:
  profiles:
    active: dev

scheduler:
  enabled: true
  tasks:
    cleanup:
      enabled: true
      interval_ms: 60000

startup:
  banner: true
  show_config: true
~~~

Scafra then loads `application-dev.yaml` or `application-dev.properties` after
the base configuration. If no active profile is configured, `default` is used.

Services can register a task with `register_scheduled_task!("cleanup", 60000,
CleanupService::cleanup)`. The task is disabled unless `scheduler.enabled` is
`true`; each task can override its interval or be disabled independently.

## HTTP performance comparison

The following local HTTP load measurements compare Scafra with Spring Boot
and Quarkus on the same small endpoint:

~~~text
GET /bench/hello/Ada -> 200 text/plain, body "Hello, Ada!"
~~~

The test used HTTP/1.1 keep-alive connections over loopback. A shared Rust
client checked every response status and body, warmed each server for 5 seconds,
then measured for 15 seconds. It ran each server separately at 32 and 128
concurrent connections. The measured order was Quarkus, Scafra, Spring Boot.
Scafra used a generated controller and the standard middleware layers, built
with Thin LTO and one codegen unit. Spring Boot used Spring MVC with Tomcat;
Quarkus used Quarkus REST in JVM mode. Their normal production builds and
runtime defaults were used. The tested versions were Spring Boot 4.1.1 and
Quarkus 3.40.1.

![HTTP load comparison chart showing throughput and p99 latency](docs/images/framework-comparison.svg)

| Concurrent connections | Framework | Throughput (requests/s) | p50 (ms) | p95 (ms) | p99 (ms) |
| ---: | --- | ---: | ---: | ---: | ---: |
| 32 | Scafra | 176,792 | 0.170 | 0.325 | 0.432 |
| 32 | Quarkus | 162,371 | 0.173 | 0.396 | 0.677 |
| 32 | Spring Boot | 110,031 | 0.281 | 0.471 | 0.611 |
| 128 | Scafra | 212,338 | 0.564 | 1.052 | 1.331 |
| 128 | Quarkus | 187,841 | 0.606 | 1.418 | 1.991 |
| 128 | Spring Boot | 127,468 | 0.981 | 1.621 | 2.010 |

In these runs, Scafra handled about 9% more requests per second than Quarkus
at 32 connections and 13% more at 128. Its p99 latency was also lower in both
runs. Compared with Spring Boot, Scafra handled about 61% and 67% more requests
per second at the two loads. These results indicate how this simple endpoint
performed on the test machine; they do not establish performance for every
application or deployment.

The test machine's hardware and exact toolchain versions were not recorded, and
the measurements are single local runs at each connection level. Treat them as
an initial comparison rather than a reproducible performance claim. They do
not measure startup time, memory use, TLS, database access, native-image mode,
or application-specific work. This repository contains the Scafra libraries
and the written results only; the temporary benchmark apps and load-test code
are not part of the project.

## Project layout

Applications generated by the CLI use compile-time source discovery:

~~~text
my-app/
├── Cargo.toml
├── src/resources/application.yaml
├── README.md
├── tests/
└── src/
    ├── main.rs
    ├── main/
    │   ├── controllers/
    │   ├── services/
    │   ├── repositories/
    │   ├── beans/
    │   ├── config/
    │   ├── models/
    │   └── errors/
    └── resources/
~~~

The monolith starter additionally includes a bounded `src/main/modules/catalog/`
module; the other starters do not create that directory.

'#[scafra::main]' discovers the module tree during compilation, so new Rust files
under 'src/main/' participate without adding manual 'mod' declarations to
'main.rs'. Rust visibility and imports remain normal and explicit inside those
files. Public graph providers and their dependencies are resolved at startup
through typed constructors. Existing components outside the graph continue to
use their registered default construction path. Applications with an
application-owned build pipeline can also use 'scafra_build::discover_graph'.

The CLI writes fixed, inspectable files: a Cargo manifest, configuration
defaults under `src/resources/`, a `#[scafra::main]` entry point, standard
responsibility directories under `src/main/`, and the shape-specific sample.
API and monolith starters add an application-level Serde dependency for their
JSON models; web and service starters do not.

For Spring-style lifecycle extension points, annotate a struct with
'#[post_processor]' and implement 'BeanPostProcessor'. It receives typed
component metadata before and after initialization. The application context
is intentionally typed metadata plus lifecycle orchestration, rather than a
global 'Any' service locator.

## Guarantees, failures, and escape hatches

Starter selection and validation happen in the CLI before project generation.
Unknown kinds are rejected, existing destinations—including dangling
symlinks—are never overwritten, and the destination name is normalized for
Cargo package naming. Generated source then receives ordinary Rust compiler
and macro diagnostics. At runtime the current runner loads and validates the
supported configuration sources, initializes foundation logging, registers
the generated routes, and uses Scafra's current tracing, body-limit, and
graceful-shutdown behavior. A generated startup failure emits the bounded
`application_startup_failed` event with an `error_kind` of `configuration`,
`address`, `lifecycle`, or `web`; it does not print arbitrary provider or
configuration error text. Direct callers of `scafra::run` still receive the
typed `StartupError` for application-owned handling.

Generation writes the complete project into a temporary sibling directory and
moves it into place only after every file has been written. A filesystem error
while staging removes that temporary directory and leaves the requested
destination absent. The destination is checked again immediately before the
move, and the completed directory is published with an atomic no-replace move
on Linux, macOS, and Windows, so a destination created concurrently is
preserved. Other Rust targets use `std::fs::rename`, whose replacement behavior
depends on the operating system. The repository's focused CLI and
non-listener integration checks cover this preflight and compilation behavior.
The live route/shutdown smoke remains a
release-gate check in an environment that permits loopback listeners.

The convention is optional. Edit or replace the generated controllers,
services, providers, and configuration; use `build_router()` and the
`scafra::web::axum`/Tower re-exports for lower-level HTTP composition; or
replace `#[scafra::main]` with an application-owned Axum/Tokio entry point when
you need a custom server boundary. That entry point must declare the application
modules manually so static route registrations remain linked. Applications
that explicitly use `scafra_build::discover` can instead include its generated
module bridge. Tokio is not a
stable public Scafra re-export, so an application-owned Tokio entry point should
declare `tokio` directly. Axum can use the `scafra::web::axum` re-export or a
direct `axum` dependency. A custom server-builder API is not part of the
current starter contract. Replacing `#[scafra::main]` transfers configuration,
startup, and graceful-shutdown ownership to the application; `build_router()`
only builds the router and does not perform that orchestration.

Starter generation and typed startup composition are current. The standard
'#[scafra::main]' path constructs controllers with their injected services and
beans before serving routes.
Runtime filesystem scanning, reflection-based discovery, a global mutable
container, and string-key dependency lookup are **Rejected for now** as the
primary architecture.

## Documentation

Start with the [documentation index](docs/README.md). It contains the current
architecture, getting-started instructions, generated-project guide,
dependency-graph guide, and contribution instructions.

## Workspace commands

~~~bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
~~~

The CLI is available from this workspace:

~~~bash
cargo run -p scafra-cli -- new my-app
cargo run -p scafra-cli -- check
cargo run -p scafra-cli -- dev
~~~

`scafra dev` is the development watcher. Run it once from the application
directory; it watches `src/`, `tests/`, `examples/`, `Cargo.toml`, and
`Cargo.lock`, then restarts the application when code or configuration changes.
Cargo's incremental compilation means dependencies are reused and rebuilt only
when the manifest or lockfile requires it.

When run from this checkout, 'scafra new' writes local path dependencies so the
generated project compiles immediately. Its manifest also declares an empty
`[workspace]` table, so creating the project inside this checkout does not make
it an unlisted member of Scafra's workspace. A packaged CLI uses the published
`0.1` dependency fallback.

## Configuration

The standard runner loads supported files from the current directory. The
precedence is defaults, base YAML/YML/properties files, profile
YAML/YML/properties files, sorted `SCAFRA_*` environment variables, then
explicit loader overrides:

~~~yaml
server:
  host: 127.0.0.1
  port: 8080
  # Optional; unset keeps the historical unlimited processing time.
  timeout: 30 # optional request-processing deadline in seconds
logging:
  level: info
  backtrace: off
~~~

The equivalent focused properties form is:

~~~properties
server.host=127.0.0.1
server.port=8080
server.timeout=30
logging.level=info
logging.backtrace=off
~~~

Set `SCAFRA_PROFILE=test` to activate the matching YAML, YML, and properties
profile files. For example, `SCAFRA_SERVER_PORT=9000 cargo run`
changes the listening port without changing source code. `RUST_LOG` remains an
explicit advanced tracing-filter override; invalid directives fall back to the
typed `logging.level` without echoing the directive.

`SCAFRA_PROFILE` selects the profile before files are merged. It takes
precedence over `scafra.profiles.active` in the base file; if neither is set,
Scafra uses the `default` profile. Profile files are applied in YAML, YML, then
properties order. Environment names after the `SCAFRA_` prefix map to
configuration paths. The historical form `SCAFRA_SERVER_PORT` resolves to
`server.port` when the serialized schema has one matching path. Use a double
underscore (`__`) to mark a path boundary explicitly; single underscores then
remain part of a field name. For example, `SCAFRA_SECURITY__BEARER_TOKEN` sets
`security.bearer_token`, and
`SCAFRA_SCHEDULER__TASKS__CLEANUP__INTERVAL_MS` sets
`scheduler.tasks.cleanup.interval_ms`. A historical single-underscore name is
accepted only when it resolves to one schema path. Unknown, ambiguous, or
duplicate environment overrides fail loading. Matching variables are
processed in sorted name order. `SCAFRA_PROFILE` selects the profile before
files are merged; the runner's `SCAFRA_DEV_CHILD` and `SCAFRA_DEV_RELOAD`
controls are not configuration overrides. `ConfigLoader::profile` selects a
profile explicitly in application code, while `override_value` is the final
per-key override for programmatic callers. Missing candidate files are
optional, but unreadable, malformed, or incorrectly typed values fail loading.

The standard runner loads and validates `ScafraConfig` before initializing
logging, discovering application components, or binding the listener. A
derived `Config` can mark `String` and `Option<T>` fields as required with
`#[config(required)]`; blank strings and absent options then fail during
`load_validated` or typed `Properties::load`. Built-in validation errors expose
the configuration key and safe reason through `ConfigError::validation_details`;
the general `Display` and `Debug` forms stay redacted. Errors from custom
validators are redacted by default; a custom validator should expose details
only when those details are known not to contain configured values. Startup
logs report a bounded error category and never format arbitrary configuration
errors.

For development, keep safe defaults in `application.yaml`, use the `dev`
profile for local-only settings, and pass secrets through environment
variables. For tests, use `SCAFRA_PROFILE=test` or a test-specific
`ConfigLoader::root` and `env_prefix`; restore process environment variables
after each test. For production, select an explicit profile, inject secrets
from the deployment environment or secret manager, and avoid committing secret
values to configuration files. Scafra does not load `.env` files implicitly.
Environment overrides are read from the process environment at load time.

`server.timeout` (also `SCAFRA_SERVER_TIMEOUT`) sets an optional
request-processing deadline in seconds. It must be greater than zero
when set. When the deadline expires, Scafra cancels the in-flight request
future and returns `504 Gateway Timeout` with an empty body. The deadline
covers security middleware, request body extraction, and handler processing
until the response is created. It does not cover establishing a connection or
receiving request headers, and it ends when the handler returns the response;
streaming response bodies can continue afterward. Cancellation does not undo
external side effects the handler already performed. An unset value preserves
the existing no-deadline behavior. Scafra uses 504 to identify this server-side
processing deadline; 408 describes the server waiting for the client to finish
sending its request.

## Crates

* 'scafra' is the published facade package, with Rust library name
  `scafra` and dependency alias `scafra` for application developers.
* 'scafra-core' contains framework-neutral lifecycle and metadata types.
* 'scafra-foundation' contains the framework-neutral typed logging and backtrace
  policy used by runtime consumers; it depends on no Scafra crate.
* 'scafra-macros' contains the procedural macros and compile-failure tests.
* 'scafra-web' owns Axum, Tokio, Tower, route registration, and graceful
  shutdown while consuming foundation logging.
* 'scafra-config' provides typed YAML/YML/properties loading with profile,
  environment, and explicit override precedence.
* 'scafra-build' generates the convention-based Rust module tree from
  'build.rs'.
* 'scafra-cli' provides 'new', 'dev', and 'check'; it installs the
  `scafra` executable.

The crates.io package for the application facade is `scafra`; the
dependency is aliased as `scafra` so application code can keep using
`use scafra::prelude::*`. After publication, install the CLI with
`cargo install scafra-cli`.

See [docs/publishing.md](docs/publishing.md) for the GitHub Actions setup and
release tag process.

See [docs/architecture.md](docs/architecture.md) for the dependency graph and
design boundaries. Generated behavior is described in
[docs/generated-code.md](docs/generated-code.md).

## Security defaults

The default listener binds to '127.0.0.1:8080', not all interfaces. Scafra
installs a 1 MiB default request body limit and request tracing without logging
request bodies.

## License

MIT OR Apache-2.0.
