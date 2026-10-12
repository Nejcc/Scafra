use crate::{
    binding::{self, Protection},
    snapshot::BootUiSnapshot,
};
use axum::{extract::State, response::Html, routing::get, Json, Router};

pub(crate) fn router(snapshot: BootUiSnapshot, protection: Protection) -> Router {
    let dashboard = Router::new()
        .route("/", get(index))
        .route("/api/config", get(config_snapshot))
        .with_state(snapshot);
    binding::protect_router(dashboard, protection)
}

async fn config_snapshot(State(snapshot): State<BootUiSnapshot>) -> Json<BootUiSnapshot> {
    Json(snapshot)
}

async fn index() -> Html<&'static str> {
    Html(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Scafra BootUI</title>
  <style>
    :root { color-scheme: dark; font-family: Inter, system-ui, sans-serif; }
    body { margin: 0; background: #111827; color: #f9fafb; }
    header { padding: 28px 36px; background: linear-gradient(135deg,#241205,#7c2d12); }
    h1 { margin: 0 0 6px; color: #fb923c; }
    main { padding: 28px 36px; display: grid; gap: 18px; grid-template-columns: repeat(auto-fit,minmax(240px,1fr)); }
    section { padding: 20px; border: 1px solid #374151; border-radius: 14px; background: #1f2937; }
    code, pre { color: #fdba74; white-space: pre-wrap; }
    .muted { color: #9ca3af; }
  </style>
</head>
<body>
  <header><h1>🔥 Scafra BootUI</h1><div class="muted">Local application operations dashboard</div></header>
  <main>
    <section><h2>Application</h2><pre id="config">Loading safe dashboard settings...</pre></section>
    <section><h2>Endpoints</h2><p><a id="health" href="#">Health</a></p><p><a id="live" href="#">Live</a></p><p><a id="ready" href="#">Ready</a></p><p><a id="metrics" href="#">Metrics</a></p></section>
    <section><h2>Status</h2><p class="muted">Use the actuator endpoints for live health data.</p></section>
  </main>
  <script>
    const base = window.location.pathname.replace(/\/$/, '');
    fetch(base + '/api/config').then(response => response.json()).then(value => {
      document.getElementById('config').textContent = JSON.stringify(value, null, 2);
      const server = `${window.location.protocol}//${value.server.host.includes(':') ? '[' + value.server.host + ']' : value.server.host}:${value.server.port}`;
      document.getElementById('health').href = server + '/health';
      document.getElementById('live').href = server + '/live';
      document.getElementById('ready').href = server + '/ready';
      document.getElementById('metrics').href = server + '/metrics';
    }).catch(error => document.getElementById('config').textContent = String(error));
  </script>
</body>
</html>"##,
    )
}
