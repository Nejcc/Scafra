use std::sync::{Mutex, OnceLock};

use scafra::{bean, service};

static EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();

mod external_client_library {
    use std::sync::Mutex;

    pub struct Client {
        _connection_state: Mutex<usize>,
        _endpoint: &'static str,
        _credentials: String,
    }

    impl Client {
        pub fn new(endpoint: &'static str, credentials: String) -> Self {
            Self {
                _connection_state: Mutex::new(1),
                _endpoint: endpoint,
                _credentials: credentials,
            }
        }
    }
}

fn record(event: &'static str) {
    EVENTS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("event log should not be poisoned")
        .push(event);
}

pub(crate) fn events() -> Vec<&'static str> {
    EVENTS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("event log should not be poisoned")
        .clone()
}

pub(crate) fn reset_events() {
    EVENTS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("event log should not be poisoned")
        .clear();
}

pub struct RuntimeConfig(&'static str);

pub struct Credentials(String);

/// An application-owned wrapper around a client supplied by another crate.
pub struct ExternalClient {
    _inner: external_client_library::Client,
}

impl Default for ExternalClient {
    fn default() -> Self {
        Self {
            _inner: external_client_library::Client::new(
                "default implementation",
                String::new(),
            ),
        }
    }
}

// The graph does not require Bean. This remains available to applications
// that use the optional marker in their own trait bounds.
impl scafra::Bean for ExternalClient {}

#[bean]
pub fn runtime_config() -> RuntimeConfig {
    record("runtime_config");
    RuntimeConfig("https://service.invalid")
}

#[bean]
pub fn credentials() -> Credentials {
    record("credentials");
    Credentials("configured at startup".to_owned())
}

#[bean]
pub fn external_client(
    config: RuntimeConfig,
    credentials: Credentials,
) -> ExternalClient {
    record("external_client");
    ExternalClient {
        _inner: external_client_library::Client::new(config.0, credentials.0),
    }
}

#[service]
pub struct ClientService {
    _client: ExternalClient,
}
