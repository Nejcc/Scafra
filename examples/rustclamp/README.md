# Scafra + RustClamp

An opt-in example that mounts a Scafra-generated Axum router as one RustClamp
HTTP contribution and adds a separate public `/health` route. Scafra still
constructs its controllers and services; RustClamp validates and composes the
HTTP contributions. One Tokio runtime serves the result.

This package has its own workspace. It does not change Scafra's workspace
dependencies, default runner or Rust 1.82 minimum. The example requires Rust
1.96.1 or later because RustClamp currently requires that version.

## Set up the prototype components

RustClamp's component crates are unpublished, and its HTTP manifest depends on
Core through `../core`. Use sibling component checkouts rather than a standalone
Git dependency on HTTP. Starting in the directory containing your Scafra checkout:

```sh
mkdir -p rustclamp
git clone https://github.com/rustclamp/core rustclamp/core
git -C rustclamp/core checkout 5b5fe2bb7f23468422234ea58d1c470fde3c1b69
git clone https://github.com/rustclamp/http rustclamp/http
git -C rustclamp/http checkout 071708827f973ac1031a9901bd53d6d1437e5dbe
```

The resulting layout must be:

```text
working-directory/
  scafra/
    crates/
    examples/rustclamp/
  rustclamp/
    core/
    http/
```

The manifest's local paths preserve a single Core crate identity for the
application and HTTP adapter. When RustClamp publishes portable packages, these
paths can be replaced with registry dependencies.

## Run and test

From the Scafra checkout:

```sh
cargo fmt --manifest-path examples/rustclamp/Cargo.toml -- --check
cargo test --manifest-path examples/rustclamp/Cargo.toml
cargo clippy --manifest-path examples/rustclamp/Cargo.toml --all-targets -- -D warnings
cargo run --manifest-path examples/rustclamp/Cargo.toml
```

The program loads Scafra configuration and honors `SCAFRA_PROFILE`. With defaults
and security disabled, request:

```sh
curl http://127.0.0.1:8080/scafra/api/hello/Alice
# Hello, Alice!
curl http://127.0.0.1:8080/health
# ok
```

Use the configured server host and port if you change them. Configured Scafra
authentication applies inside `/scafra`; supply the configured credentials when
enabled. `/health` is intentionally public and sits outside that layer.

The tests exercise a generated controller and its service, the separate health
route, bearer authentication after mounting, and conflicting contribution
rejection. Mounting under `/scafra` changes external URLs; choose a root mount
deliberately if your application needs its original paths.

## Startup and shutdown boundaries

The example validates composition and binds the listener before starting
Scafra's application lifecycle. It starts Scafra's configured scheduler and stops
it before lifecycle cleanup. Ctrl-C initiates RustClamp's bounded ten-second
HTTP drain. Signal-registration and serving errors still pass through cleanup;
cleanup failure does not hide an earlier serving error.

This custom entry point does not reproduce the complete default Scafra runner:
it has no SIGTERM handling, startup banner or development supervisor, does not
start optional extensions such as BootUI, and uses its own fixed HTTP drain
timeout instead of Scafra's HTTP shutdown policy. Scafra's actuator and security
configuration are explicitly passed to its router builder. Applications that
need the omitted runner behavior must wire it in explicitly.

The shared boundary is an Axum 0.8 router. This example does not bridge Scafra's
Default-based service construction to RustClamp capability resolution or move
Scafra scheduled tasks into RustClamp's scheduler.
