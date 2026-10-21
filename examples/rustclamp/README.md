# Scafra + RustClamp (experimental)

> **Experimental.** RustClamp requires Rust 1.96.1, a newer toolchain than
> Scafra's minimum, so this example lives outside Scafra's workspace and may
> change as RustClamp evolves.

An opt-in example that mounts a Scafra-generated Axum router as one RustClamp
HTTP contribution and adds a separate public `/health` route. Scafra still
constructs its controllers and services; RustClamp validates and composes the
HTTP contributions. Scafra's standard runner serves the result.

This package has its own workspace. It does not change Scafra's workspace
dependencies, default runner or Rust 1.82 minimum. The example requires Rust
1.96.1 or later because RustClamp currently requires that version.

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
route, bearer authentication after mounting, conflicting contribution
rejection, and startup failures for invalid composition and configuration. Mounting under `/scafra` changes external URLs; choose a root mount
deliberately if your application needs its original paths.

## Startup and shutdown boundaries

The example passes its composed router to `scafra::run_with_router`, so Scafra's
standard runner owns configuration loading, `SCAFRA_PROFILE`, logging, the
application lifecycle, the scheduler, signal handling and HTTP shutdown policy.
Composition runs after configuration loads and before the lifecycle starts; a
composition failure is reported as a `web` startup error and nothing is
started. Scafra serves the supplied router as-is, so its security layer applies
only inside `/scafra`.

The shared boundary is an Axum 0.8 router. This example does not bridge Scafra's
Default-based service construction to RustClamp capability resolution or move
Scafra scheduled tasks into RustClamp's scheduler.
