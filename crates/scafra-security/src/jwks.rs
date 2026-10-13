//! Bounded OIDC discovery, JWKS retrieval, and per-router key-set caching.

use crate::config::{valid_provider_url, JwtConfig};
use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, RwLock},
    time::Instant,
};

const CACHE_LIFETIME: Duration = Duration::from_secs(5 * 60);
const REFRESH_COOLDOWN: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub(super) struct JwksCache(Arc<JwksCacheInner>);

struct JwksCacheInner {
    state: RwLock<CacheState>,
    refresh_gate: Mutex<()>,
    client: Option<reqwest::Client>,
    cache_lifetime: Duration,
    refresh_cooldown: Duration,
    request_timeout: Duration,
}

#[derive(Default)]
struct CacheState {
    keys: Option<CachedJwkSet>,
    last_attempt: Option<Instant>,
    last_unknown_kid_refresh: Option<Instant>,
}

struct CachedJwkSet {
    jwks: JwkSet,
    fetched_at: Instant,
}

#[derive(Clone, Deserialize)]
pub(super) struct JwkSet {
    pub(super) keys: Vec<JwkKey>,
}

#[derive(Clone, Deserialize)]
pub(super) struct JwkKey {
    pub(super) kid: Option<String>,
    pub(super) kty: String,
    pub(super) n: Option<String>,
    pub(super) e: Option<String>,
    pub(super) alg: Option<String>,
    #[serde(rename = "use")]
    pub(super) public_key_use: Option<String>,
    pub(super) key_ops: Option<Vec<String>>,
}

impl JwksCache {
    pub(super) fn new() -> Self {
        Self::with_policy(CACHE_LIFETIME, REFRESH_COOLDOWN, REQUEST_TIMEOUT)
    }

    fn with_policy(
        cache_lifetime: Duration,
        refresh_cooldown: Duration,
        request_timeout: Duration,
    ) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(request_timeout.min(CONNECT_TIMEOUT))
            .timeout(request_timeout)
            // Provider redirects can change the configured destination. Requiring
            // an explicit configured URL keeps discovery and key retrieval bounded.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok();
        Self(Arc::new(JwksCacheInner {
            state: RwLock::new(CacheState::default()),
            refresh_gate: Mutex::new(()),
            client,
            cache_lifetime,
            refresh_cooldown,
            request_timeout,
        }))
    }
}

/// Return a matching cached key, refreshing at most once per cooldown after an
/// unknown key. Expired keys are never used while a refresh is pending or has
/// failed.
pub(super) async fn get_key(cache: &JwksCache, config: &JwtConfig, kid: &str) -> Option<JwkKey> {
    if kid.is_empty() {
        return None;
    }

    if let Some(key) = cached_key(cache, kid, Instant::now()).await {
        return Some(key);
    }

    // The lock is application-owned and held only around refresh coordination.
    // A waiting request rechecks state after acquiring it, so an in-flight
    // refresh serves all concurrent misses without a provider-call stampede.
    let _refresh = cache.0.refresh_gate.lock().await;
    let now = Instant::now();
    if let Some(key) = cached_key(cache, kid, now).await {
        return Some(key);
    }

    let refresh_for_unknown_kid = {
        let state = cache.0.state.read().await;
        if is_fresh(state.keys.as_ref(), now, cache.0.cache_lifetime) {
            if state
                .last_unknown_kid_refresh
                .is_some_and(|last| within(last, now, cache.0.refresh_cooldown))
            {
                return None;
            }
            true
        } else {
            if state
                .last_attempt
                .is_some_and(|last| within(last, now, cache.0.refresh_cooldown))
            {
                return None;
            }
            false
        }
    };

    {
        let mut state = cache.0.state.write().await;
        state.last_attempt = Some(now);
        if refresh_for_unknown_kid {
            state.last_unknown_kid_refresh = Some(now);
        }
    }

    let jwks = fetch_jwks(cache, config).await?;
    let key = find_key(&jwks, kid).cloned();
    let mut state = cache.0.state.write().await;
    state.keys = Some(CachedJwkSet {
        jwks,
        fetched_at: Instant::now(),
    });
    key
}

async fn cached_key(cache: &JwksCache, kid: &str, now: Instant) -> Option<JwkKey> {
    let state = cache.0.state.read().await;
    if !is_fresh(state.keys.as_ref(), now, cache.0.cache_lifetime) {
        return None;
    }
    state
        .keys
        .as_ref()
        .and_then(|cached| find_key(&cached.jwks, kid))
        .cloned()
}

fn is_fresh(cached: Option<&CachedJwkSet>, now: Instant, lifetime: Duration) -> bool {
    cached.is_some_and(|cached| {
        now.checked_duration_since(cached.fetched_at)
            .is_some_and(|age| age < lifetime)
    })
}

fn within(last: Instant, now: Instant, cooldown: Duration) -> bool {
    now.checked_duration_since(last)
        .is_some_and(|elapsed| elapsed < cooldown)
}

fn find_key<'a>(jwks: &'a JwkSet, kid: &str) -> Option<&'a JwkKey> {
    jwks.keys.iter().find(|key| key.kid.as_deref() == Some(kid))
}

async fn fetch_jwks(cache: &JwksCache, config: &JwtConfig) -> Option<JwkSet> {
    let client = cache.0.client.as_ref()?;
    let operation = async {
        let uri = if let Some(uri) = config.jwk_set_uri.as_deref() {
            if !valid_provider_url(uri, false) {
                return None;
            }
            uri.to_owned()
        } else {
            let issuer = config.issuer_uri.as_deref()?;
            if !valid_provider_url(issuer, true) {
                return None;
            }
            let metadata_uri = format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            );
            let metadata = fetch_json(client, &metadata_uri).await?;
            let jwks_uri = metadata.get("jwks_uri")?.as_str()?;
            if !valid_provider_url(jwks_uri, false) {
                return None;
            }
            jwks_uri.to_owned()
        };

        let bytes = fetch_bytes(client, &uri).await?;
        let jwks = serde_json::from_slice::<JwkSet>(&bytes).ok()?;
        validate_jwks(&jwks).then_some(jwks)
    };

    tokio::time::timeout(cache.0.request_timeout, operation)
        .await
        .ok()?
}

async fn fetch_json(client: &reqwest::Client, uri: &str) -> Option<Value> {
    let bytes = fetch_bytes(client, uri).await?;
    serde_json::from_slice(&bytes).ok()
}

async fn fetch_bytes(client: &reqwest::Client, uri: &str) -> Option<Vec<u8>> {
    let mut response = client.get(uri).send().await.ok()?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64)
    {
        return None;
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len().saturating_add(chunk.len()) > MAX_PROVIDER_RESPONSE_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

fn validate_jwks(jwks: &JwkSet) -> bool {
    let mut key_ids = HashSet::new();
    for key in &jwks.keys {
        if let Some(kid) = key.kid.as_deref() {
            if kid.is_empty() || !key_ids.insert(kid) {
                return false;
            }
        }
        if key.kty == "RSA" {
            let (Some(n), Some(e)) = (key.n.as_deref(), key.e.as_deref()) else {
                return false;
            };
            if n.is_empty() || e.is_empty() || DecodingKey::from_rsa_components(n, e).is_err() {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::SecurityConfig, jwt};
    use axum::{routing::get, Json, Router};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::json;
    use tokio::net::TcpListener;

    const ORIGINAL_N: &str = "tEaY79VFcn9pvrOHgFR6RVjud03RgrxyuUOlJXJEF7-VuOsRk8350rygE1dQ0eQo-O6J3JxR5DzZh9Bd6zCQ2_7Jp6BWo26b6x0ZtnLKGQkezc5rbshS9y2r7SnyCsrAS9G_IGQpy2pnRvQq0_BlSnIo3qH8MarecboZl-sJ4PLDDNioC3cD6HkF5rCvyh_-YZGMO8t8DfPJX0erTTnE8ss69-WQq6BBA6HzVx2HcYpaP_hg_WN30ozenvTYGpCCdAAtzpuefpZlQn-jI5ZIvnAsBG7627rTH91GhHkbrVAZPPkQq5-G1zl6qfoD7tVONGRLCucx8qZvKzkHesiG6w";
    const ROTATED_N: &str = "sQJJVWGz6HNs8fa-c7is8E426saUv1aD9BPLsIR2xwVYcAG8-SlTpI94-Rhusx8uPthD_wC-zv7QHlMoR2usXpXIeBKi0ROhISRpr5s2ciYa3NLWIiDBg6QdB0pzoPuVtBoxLzQaOLndOsO9-l3eJ35ePKor9suYy238x24IcbkJyBwT9VpWzTyOfDmH1THsPCiLyH_Su37vJdLX0z10-Gb39BNHlQyELEo53PsY_zt5WPut8sfoEaLV5-7IhhZ2U4jn6qjTSITBPB4rSkA4LRjBvSQM6ja9SlEcKXmv25h7KLldQruO5kbVZnGmUvfD7910lwpz1UmiPEORAuVJCQ";
    const ORIGINAL_PRIVATE_KEY: &[u8] = include_bytes!("../tests/fixtures/rsa-private.pem");
    const ROTATED_PRIVATE_KEY: &[u8] = include_bytes!("../tests/fixtures/rsa-rotated-private.pem");

    #[test]
    fn cache_lifetime_and_cooldown_have_strict_boundaries() {
        let start = Instant::now();
        let cached = CachedJwkSet {
            jwks: JwkSet { keys: Vec::new() },
            fetched_at: start,
        };

        assert!(is_fresh(Some(&cached), start, Duration::from_secs(10)));
        assert!(is_fresh(
            Some(&cached),
            start + Duration::from_secs(9),
            Duration::from_secs(10)
        ));
        assert!(!is_fresh(
            Some(&cached),
            start + Duration::from_secs(10),
            Duration::from_secs(10)
        ));
        assert!(within(
            start,
            start + Duration::from_secs(29),
            Duration::from_secs(30)
        ));
        assert!(!within(
            start,
            start + Duration::from_secs(30),
            Duration::from_secs(30)
        ));
    }

    #[test]
    fn malformed_rsa_jwks_and_duplicate_key_ids_are_rejected() {
        let malformed = JwkSet {
            keys: vec![JwkKey {
                kid: Some("key".to_owned()),
                kty: "RSA".to_owned(),
                n: Some("invalid".to_owned()),
                e: Some("AQAB".to_owned()),
                alg: None,
                public_key_use: None,
                key_ops: None,
            }],
        };
        assert!(!validate_jwks(&malformed));

        let duplicate = JwkSet {
            keys: vec![
                JwkKey {
                    kid: Some("key".to_owned()),
                    kty: "RSA".to_owned(),
                    n: Some("n".to_owned()),
                    e: Some("e".to_owned()),
                    alg: None,
                    public_key_use: None,
                    key_ops: None,
                },
                JwkKey {
                    kid: Some("key".to_owned()),
                    kty: "oct".to_owned(),
                    n: None,
                    e: None,
                    alg: None,
                    public_key_use: None,
                    key_ops: None,
                },
            ],
        };
        assert!(!validate_jwks(&duplicate));
    }

    #[tokio::test]
    async fn expired_cached_key_is_not_used_after_refresh_failure() {
        let cache = JwksCache::with_policy(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_millis(20),
        );
        let now = Instant::now();
        {
            let mut state = cache.0.state.write().await;
            state.keys = Some(CachedJwkSet {
                jwks: JwkSet {
                    keys: vec![JwkKey {
                        kid: Some("stale".to_owned()),
                        kty: "RSA".to_owned(),
                        n: Some("n".to_owned()),
                        e: Some("e".to_owned()),
                        alg: None,
                        public_key_use: None,
                        key_ops: None,
                    }],
                },
                fetched_at: now - Duration::from_secs(2),
            });
        }
        let config = JwtConfig {
            enabled: true,
            jwk_set_uri: Some("http://127.0.0.1:1/keys".to_owned()),
            ..JwtConfig::default()
        };

        assert!(get_key(&cache, &config, "stale").await.is_none());
    }

    #[tokio::test]
    async fn provider_request_times_out_within_the_configured_bound() {
        use axum::{body::Body, http::Response, routing::get, Router};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/keys",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(150)).await;
                Response::new(Body::from("{}"))
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let cache = JwksCache::with_policy(
            Duration::from_secs(30),
            Duration::from_secs(1),
            Duration::from_millis(20),
        );
        let config = JwtConfig {
            enabled: true,
            jwk_set_uri: Some(format!("http://{address}/keys")),
            ..JwtConfig::default()
        };

        let started = Instant::now();
        assert!(get_key(&cache, &config, "key").await.is_none());
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn expired_cache_replaces_a_same_kid_key_before_accepting_tokens() {
        let jwks_uri = serve_jwks(json!({
            "keys": [{"kid":"rotating-key","kty":"RSA","n":ROTATED_N,"e":"AQAB"}]
        }))
        .await;
        let cache = JwksCache::with_policy(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_millis(500),
        );
        {
            let mut state = cache.0.state.write().await;
            state.keys = Some(CachedJwkSet {
                jwks: JwkSet {
                    keys: vec![JwkKey {
                        kid: Some("rotating-key".to_owned()),
                        kty: "RSA".to_owned(),
                        n: Some(ORIGINAL_N.to_owned()),
                        e: Some("AQAB".to_owned()),
                        alg: None,
                        public_key_use: None,
                        key_ops: None,
                    }],
                },
                fetched_at: Instant::now() - Duration::from_secs(2),
            });
        }
        let security = SecurityConfig {
            jwt: JwtConfig {
                enabled: true,
                issuer_uri: Some("https://issuer.test".to_owned()),
                jwk_set_uri: Some(jwks_uri),
                audiences: vec!["api".to_owned()],
                ..JwtConfig::default()
            },
            ..SecurityConfig::default()
        };
        let rotated = signed_token(ROTATED_PRIVATE_KEY);
        assert!(jwt::validate_jwt(&security, &cache, &rotated)
            .await
            .is_some());

        let removed = signed_token(ORIGINAL_PRIVATE_KEY);
        assert!(jwt::validate_jwt(&security, &cache, &removed)
            .await
            .is_none());
    }

    async fn serve_jwks(document: Value) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/keys",
            get(move || {
                let document = document.clone();
                async move { Json(document) }
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{address}/keys")
    }

    fn signed_token(signing_key: &[u8]) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("rotating-key".to_owned());
        encode(
            &header,
            &json!({
                "iss":"https://issuer.test",
                "aud":"api",
                "exp":4_000_000_000_u64
            }),
            &EncodingKey::from_rsa_pem(signing_key).unwrap(),
        )
        .unwrap()
    }
}
