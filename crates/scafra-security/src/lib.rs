//! Configurable HTTP authentication and route authorization for Scafra.
//!
//! The public API stays at the crate root: [`SecurityConfig`] and related
//! configuration types describe authenticators, [`AuthorizationPolicy`] and
//! [`RouteAuthorizationMetadata`] describe route requirements, and [`layer`]
//! or [`layer_with_route_policies`] applies them to an Axum router.
//!
//! Internally, configuration, policy matching, HTTP middleware, credential
//! authentication, JWT validation, and JWKS retrieval live in separate
//! modules. Security is disabled by default. When enabled, bearer tokens,
//! HTTP Basic credentials, and JWTs validated with HS256 or RSA JWKS are
//! supported. Authentication and authorization failures are denied; malformed
//! or unsupported credentials do not grant access. Shared bearer and Basic
//! credentials authenticate requests but do not provide roles or scopes.
//! JWKS are fetched and cached once for the lifetime of the router state.
//! Credential-bearing configuration types redact tokens, usernames, passwords,
//! HMAC secrets, and issuer/JWKS URLs from their `Debug` output.

mod authentication;
mod config;
mod jwks;
mod jwt;
mod middleware;
mod policy;

pub use config::{BasicAuthConfig, JwtConfig, SecurityConfig};
pub use middleware::{layer, layer_with_route_policies};
pub use policy::{
    AuthorizationMode, AuthorizationPolicy, ControllerAuthorizationRegistration,
    RouteAuthorizationMetadata,
};
