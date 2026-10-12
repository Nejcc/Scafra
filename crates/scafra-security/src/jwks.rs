//! JWKS endpoint discovery, retrieval, and the per-router key-set cache.

use crate::config::JwtConfig;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

pub(super) type JwksCache = Arc<RwLock<Option<JwkSet>>>;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct JwkSet {
    pub(super) keys: Vec<JwkKey>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct JwkKey {
    pub(super) kid: Option<String>,
    pub(super) kty: String,
    pub(super) n: Option<String>,
    pub(super) e: Option<String>,
}

pub(super) async fn get_or_fetch(cache: &JwksCache, config: &JwtConfig) -> Option<JwkSet> {
    let cached = cache.read().await.clone();
    if let Some(jwks) = cached {
        return Some(jwks);
    }

    let jwks = fetch_jwks(config).await?;
    *cache.write().await = Some(jwks.clone());
    Some(jwks)
}

async fn fetch_jwks(config: &JwtConfig) -> Option<JwkSet> {
    let uri = if let Some(uri) = config.jwk_set_uri.as_deref() {
        uri.to_owned()
    } else {
        let issuer = config.issuer_uri.as_deref()?;
        let metadata = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        reqwest::get(metadata)
            .await
            .ok()?
            .json::<Value>()
            .await
            .ok()?
            .get("jwks_uri")?
            .as_str()?
            .to_owned()
    };
    reqwest::get(uri).await.ok()?.json::<JwkSet>().await.ok()
}
