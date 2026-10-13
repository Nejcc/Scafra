use std::path::Path;

use anyhow::Result;

use crate::{cli::ApplicationKind, filesystem::local_dependency};

pub(crate) const STANDARD_DIRECTORIES: &[&str] = &[
    "src",
    "src/main",
    "src/main/controllers",
    "src/main/services",
    "src/main/repositories",
    "src/main/beans",
    "src/main/config",
    "src/main/models",
    "src/main/errors",
    "src/resources",
    "tests",
];

pub(crate) fn template_files(
    project: &Path,
    package_name: &str,
    kind: ApplicationKind,
) -> Result<Vec<(&'static str, String)>> {
    let scafra_dependency =
        local_dependency(project, "scafra")?.unwrap_or_else(|| "scafra = \"0.1\"".to_owned());
    let serde_dependency = if kind.needs_serde() {
        "serde = { version = \"1\", features = [\"derive\"] }\n"
    } else {
        ""
    };
    let cargo_toml = format!(
        "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n\n[dependencies]\n{scafra_dependency}\n{serde_dependency}\n[profile.release]\nlto = \"thin\"\ncodegen-units = 1\n"
    );

    let mut files = vec![
        ("Cargo.toml", cargo_toml),
        (
            "src/resources/application.yaml",
            "server:\n  host: 127.0.0.1\n  port: 8080\nlogging:\n  level: info\n".to_owned(),
        ),
        (
            "src/main.rs",
            "#[scafra::main]\nasync fn main() {}\n".to_owned(),
        ),
        ("README.md", project_readme(kind)),
    ];

    match kind {
        ApplicationKind::Web => add_web_files(&mut files),
        ApplicationKind::Api => add_api_files(&mut files),
        ApplicationKind::Service => add_service_files(&mut files),
        ApplicationKind::Monolith => add_monolith_files(&mut files),
    }

    Ok(files)
}

fn add_web_files(files: &mut Vec<(&'static str, String)>) {
    files.extend([
        (
            "src/main/beans/greeting_prefix.rs",
            r#"use scafra::prelude::*;

pub struct GreetingPrefix(pub String);

#[bean]
pub fn greeting_prefix() -> GreetingPrefix {
    GreetingPrefix(String::from("Hello,"))
}
"#
            .to_owned(),
        ),
        (
            "src/main/services/hello_service.rs",
            r#"use scafra::prelude::*;

use crate::beans::greeting_prefix::GreetingPrefix;

#[service]
pub struct HelloService {
    prefix: GreetingPrefix,
}

impl HelloService {
    pub fn greet(&self, name: &str) -> String {
        format!("{} {name}!", self.prefix.0)
    }
}
"#
            .to_owned(),
        ),
        (
            "src/main/controllers/hello_controller.rs",
            r#"use scafra::prelude::*;

use std::sync::Arc;

use crate::services::hello_service::HelloService;

#[controller("/")]
pub struct HelloController {
    service: Arc<HelloService>,
}

#[routes]
impl HelloController {
    #[get("/hello/{name}")]
    pub async fn hello(&self, name: Path<String>) -> String {
        self.service.greet(&name)
    }
}

#[controller("/")]
pub struct RepeatedHelloController {
    service: Arc<HelloService>,
}

#[routes]
impl RepeatedHelloController {
    #[get("/hello-again/{name}")]
    pub async fn hello_again(&self, name: Path<String>) -> String {
        self.service.greet(&name)
    }
}
"#
            .to_owned(),
        ),
    ]);
}

fn add_api_files(files: &mut Vec<(&'static str, String)>) {
    files.extend([
        (
            "src/main/models/greeting.rs",
            r#"use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Greeting {
    pub message: String,
}
"#
            .to_owned(),
        ),
        (
            "src/main/services/greeting_service.rs",
            r#"use scafra::prelude::*;

#[service]
pub struct GreetingService;

impl GreetingService {
    pub fn greet(&self, name: &str) -> String {
        format!("Hello, {name}!")
    }
}
"#
            .to_owned(),
        ),
        (
            "src/main/controllers/greeting_controller.rs",
            r#"use scafra::prelude::*;

use crate::models::greeting::Greeting;
use crate::services::greeting_service::GreetingService;

#[controller("/api")]
pub struct GreetingController {
    service: GreetingService,
}

#[routes]
impl GreetingController {
    #[get("/greetings/{name}")]
    pub async fn greeting(&self, name: Path<String>) -> Json<Greeting> {
        Json(Greeting {
            message: self.service.greet(&name),
        })
    }
}
"#
            .to_owned(),
        ),
    ]);
}

fn add_service_files(files: &mut Vec<(&'static str, String)>) {
    files.push((
        "src/main/controllers/health_controller.rs",
        r#"use scafra::prelude::*;

#[controller("/")]
pub struct HealthController;

#[routes]
impl HealthController {
    #[get("/health")]
    pub async fn health(&self) -> &'static str {
        "ok"
    }
}
"#
        .to_owned(),
    ));
}

fn add_monolith_files(files: &mut Vec<(&'static str, String)>) {
    files.extend([
        (
            "src/main/modules/catalog/catalog_model.rs",
            r#"use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CatalogItem {
    pub id: u32,
    pub name: String,
}
"#
            .to_owned(),
        ),
        (
            "src/main/modules/catalog/catalog_service.rs",
            r#"use scafra::prelude::*;

use super::catalog_model::CatalogItem;

#[service]
pub struct CatalogService;

impl CatalogService {
    pub fn items(&self) -> Vec<CatalogItem> {
        vec![
            CatalogItem {
                id: 1,
                name: "Notebook".to_owned(),
            },
            CatalogItem {
                id: 2,
                name: "Pen".to_owned(),
            },
        ]
    }
}
"#
            .to_owned(),
        ),
        (
            "src/main/modules/catalog/catalog_controller.rs",
            r#"use scafra::prelude::*;

use super::{
    catalog_model::CatalogItem,
    catalog_service::CatalogService,
};

#[controller("/catalog")]
pub struct CatalogController {
    service: CatalogService,
}

#[routes]
impl CatalogController {
    #[get("/items")]
    pub async fn items(&self) -> Json<Vec<CatalogItem>> {
        Json(self.service.items())
    }
}
"#
            .to_owned(),
        ),
    ]);
}

fn project_readme(kind: ApplicationKind) -> String {
    const TEMPLATE: &str = r#"# Scafra {{KIND}} application

This project was generated as {{DESCRIPTION}}. It uses the standard Scafra
facade, compile-time source discovery, and `#[scafra::main]` entry point.

The canonical application kinds are `web`, `api`, `service`, and `monolith`.
The CLI also accepts `microservice` as an alias for `service`.
Kind selection happens only while this project is generated; the application
does not receive a runtime kind setting or select components from a registry.

## Run it

```bash
cargo check
cargo run
```

In another terminal:

```bash
curl http://127.0.0.1:8080{{ROUTE}}
```

The representative route is `GET {{ROUTE}}`. The starter sample is ordinary
Rust code that you can replace or extend in `src/`.

For the example request above, the current sample response is:

```{{RESPONSE_FORMAT}}
{{RESPONSE}}
```

The generated `src/resources/application.yaml` keeps Scafra's current defaults: it binds to
`127.0.0.1:8080` and logs at `info`. `SCAFRA_SERVER_PORT=9000 cargo run` is the
existing environment override.

## What was generated

The project includes Cargo.toml and src/main.rs,
src/resources/application.yaml, tests/, and the standard controllers,
services, repositories, beans, config, models, and errors directories under
src/main/. The monolith sample also includes a bounded
src/main/modules/catalog/ module. The `#[scafra::main]` macro discovers source
modules at compile time; it does not scan the filesystem at runtime.

Cargo.toml declares an empty workspace so this project can be generated inside
another Cargo workspace without being treated as an unlisted member. It remains
an independently checkable project that can be moved elsewhere.

Cargo and Rust type-check the generated manifest and source files. The current
runner loads the configuration, registers the generated route, applies Scafra's
tracing/body-limit defaults, and follows the existing graceful-shutdown path.
Unknown kinds are rejected before generation and an existing destination is
never overwritten; ordinary Cargo, macro, configuration, and startup errors
remain visible when generated code is changed or run.

## Test and customize

Use `cargo check` and `cargo test` for the generated project. Run it and call
the representative route above for a local HTTP smoke test; JSON starters
should return `application/json`. The repository's complete starter smoke
also checks graceful shutdown, but requires an environment that permits
loopback listeners. To take control, edit or replace the generated Rust files,
use `ConfigLoader`, compose `build_router()` with the
`scafra::web::axum`/Tower re-exports, or replace `#[scafra::main]` with an
application-owned Axum/Tokio server boundary. A custom entry point must
declare the application modules manually so static route registrations remain
linked. An application that explicitly uses `scafra_build::discover` can instead
include its generated module bridge. Tokio is not a stable public Scafra
re-export, so an application-owned Tokio entry point should declare `tokio`
directly. Axum can use the `scafra::web::axum` re-export or a direct `axum`
dependency. Replacing `#[scafra::main]` transfers configuration, startup, and
graceful-shutdown ownership to the application; `build_router()` only builds
the router and does not perform that orchestration.

Starter generation and typed startup composition are **Current**. The standard
`#[scafra::main]` path composes graph values and injects controller fields
before route registration. Scafra's
standard health, liveness, readiness, and info endpoints can be enabled from
the application configuration, including Spring Boot-compatible `/actuator`
aliases. Runtime filesystem scanning, reflection-based discovery, a global
mutable container, and string-key dependency lookup are not used as the
primary architecture.
"#;
    TEMPLATE
        .replace("{{KIND}}", kind.as_str())
        .replace("{{DESCRIPTION}}", kind.description())
        .replace("{{ROUTE}}", kind.route())
        .replace("{{RESPONSE_FORMAT}}", kind.response_format())
        .replace("{{RESPONSE}}", kind.sample_response())
}
