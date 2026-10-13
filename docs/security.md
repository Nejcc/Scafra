# HTTP authentication and authorization

Scafra supports shared bearer tokens, HTTP Basic credentials, and JWT resource
server validation. Route and controller policies use the configured
authenticator; they do not introduce a second identity system.

## Crate API and modules

`scafra-security` keeps its public API at the crate root. Existing imports of
`SecurityConfig`, `BasicAuthConfig`, `JwtConfig`, authorization policy types,
registration metadata, `layer`, and `layer_with_route_policies` remain valid.
The implementation is organized by responsibility:

- `config` defines security configuration defaults and validation.
- `policy` defines controller and route requirements and matches route paths.
- `middleware` applies those decisions to Axum requests and responses.
- `authentication` parses shared bearer and Basic credentials.
- `jwt` validates signatures and standard claims and extracts roles/scopes.
- `jwks` performs OIDC discovery, retrieves RSA key sets, and refreshes a
  per-router cache.

These modules are implementation details; callers should continue using the
crate-root API. Configuration debug output redacts bearer tokens, Basic
usernames and passwords, JWT HMAC secrets, and issuer/JWKS URLs. The crate does
not log credentials or include them in authentication responses.

## Defaults

Application security is disabled by default. With `security.enabled: false`,
routes without an explicit protected policy are public. A controller or route
marked as protected makes router construction fail unless security is enabled
and at least one authenticator is configured. This prevents an annotation
from silently becoming a no-op.

With `security.enabled: true`, every route inherits a protected application
policy, including hand-written routes. `security.permit_all` keeps its existing
exact-path behavior for routes that inherit the application policy. Explicit
controller or route policies take precedence over `permit_all`.
Hand-written Axum routes use the application-wide policy; controller and
method annotations are emitted by Scafra's `#[controller]` and `#[routes]`
macros.

An application-wide bearer token, when configured, must not be empty or
whitespace-only. Typed configuration validation rejects it before the router
is built or the server starts. A valid token is retained as configured and
matched exactly; Scafra does not trim it. With application security enabled,
if no bearer token is configured, another complete authenticator is required,
such as both Basic username and password or a valid JWT configuration. Basic
Auth and JWT validation keep their existing behavior. Security configuration
errors and diagnostics do not include configured secrets, and configuration
`Debug` output redacts them.

## Controller and route policies

The controller policy is the default for each route. An omitted route policy
inherits its controller policy. Controller policies can inherit the application
policy, make all controller routes public, require authentication, or require
JWT roles and scopes:

```rust
use scafra::prelude::*;

#[controller("/catalog")]
struct CatalogController;

#[routes]
impl CatalogController {
    // Inherits the application policy.
    #[get("/items")]
    async fn items(&self) -> &'static str { "items" }
}

#[controller("/admin", roles("admin", "operator"), scopes("users.read"))]
struct AdminController;

#[routes]
impl AdminController {
    // Requires either controller role and the controller scope.
    #[get("/users")]
    async fn users(&self) -> &'static str { "users" }

    // Explicitly public for this method and path, even when global security
    // is enabled. Keep public exceptions narrow.
    #[get("/status")]
    #[public]
    async fn status(&self) -> &'static str { "ok" }
}
```

The available policy attributes are:

- `#[controller("/path", public)]`: explicitly public controller routes.
- `#[controller("/path", authenticated)]`: require a valid credential.
- `#[controller("/path", roles("admin", "operator"))]`: require at least
  one listed role. Each separate `roles(...)` group is also required.
- `#[controller("/path", scopes("items.read", "items.export"))]`: require
  every listed scope.
- On a route method, `#[public]` explicitly overrides controller and
  application policies for that HTTP method and path.
- On a route method, `#[authenticated]` requires authentication. `#[roles(...)]`
  and `#[scopes(...)]` require authentication and add requirements to the
  controller policy.

Controller and route role groups combine with AND: a token must match at least
one role in every group. Names inside one group combine with OR. Scopes combine
with AND, so every required scope must be present. `#[public]` cannot be
combined with role or scope attributes. An explicit route policy can make a
public controller route protected; it cannot remove a controller's role or
scope requirements. The only weakening override is the visibly explicit
`#[public]` exception.

Every method/path declared through `#[routes]` receives its own policy record.
Aliases declared on one handler are each protected. `HEAD` follows the matching `GET` policy,
and an undeclared method on a protected path cannot use a 405 response to skip
the path policy. A public route annotation applies only to its declared method
and exact path. Axum distinguishes a trailing slash, so `/status` and
`/status/` are separate routes.

## JWT roles and scopes

JWT signature and configured issuer, audience, expiration, and `nbf` are
validated before authorization. Scafra requires an `exp` claim. It reads roles from a
string array in `roles` or a string/string array in `role`. It reads scopes
from a space-separated `scope` string or a string array in `scp`.

For example, this JWT payload grants the `operator` role and two scopes:

```json
{
  "iss": "https://issuer.example.com",
  "aud": "scafra-api",
  "exp": 2000000000,
  "roles": ["operator"],
  "scope": "users.read users.export"
}
```

`security.jwt.required_scopes` applies to every authenticated JWT route except
routes explicitly marked public. Route and controller scope requirements add
to that list. Shared bearer tokens and Basic credentials authenticate a
request, but have no role or scope claims; they receive `403 Forbidden` when a
route requires roles or scopes.

## HTTP responses and configuration

- Missing, malformed, invalid, or expired credentials on a protected route
  return `401 Unauthorized` with a generic authentication challenge.
- Requests with more than one `Authorization` header value are malformed and
  return `401 Unauthorized`.
- A valid credential without the required role or scope returns
  `403 Forbidden`.
- The default response does not include token contents, validation details, or
  configured secrets. `hide_unauthorized: true` preserves the existing option
  to mask both authentication and authorization denials as `404 Not Found`.
- Explicitly public routes do not authenticate or inspect an `Authorization`
  header.
- Unsupported schemes and malformed credentials are rejected; failures do not
  fall back to anonymous access.

Enable JWT protection in `application.yaml` like this:

```yaml
security:
  enabled: true
  jwt:
    enabled: true
    issuer_uri: https://issuer.example.com
    audiences: [scafra-api]
    required_scopes: [api.read]
```

For development, `issuer_uri` can be replaced by an HS256 `secret`. A
`jwk_set_uri` can supply RSA public keys directly. Secrets should come from
Scafra's environment configuration rather than committed files. A JWT secret
cannot be combined with remote JWKS configuration. Provider URLs must be valid
HTTP or HTTPS URLs without URL credentials; a direct JWKS URL takes precedence
over discovery when both `issuer_uri` and `jwk_set_uri` are configured.
Scafra does not support inline or file-based static JWKS configuration.

The current validation contract keeps issuer and audience checks optional:
`issuer_uri`, when configured, is also the exact expected `iss` value, and a
non-empty `audiences` list is enforced. Empty audience entries are rejected at
configuration validation. Direct JWKS configuration can therefore be used
without issuer discovery, as before, but operators should configure both the
issuer and audience whenever the provider supplies them.

JWT algorithms are fixed by key mode. Local secrets accept HS256 only. Remote
JWKS accepts RS256, RS384, and RS512 with RSA keys. A selected RSA JWK must
match the token algorithm when its `alg` is present, permit signature use when
`use` is present, and permit verification when `key_ops` is present. The JWT
header cannot expand these algorithm sets. Signature, `exp`, `nbf`, configured
issuer, and configured audiences are checked before Scafra extracts any role or
scope claims.

## JWKS cache and rotation

Each security middleware instance owns its own JWKS cache; separate routers do
not share provider state. A successful JWKS document is cached for five minutes
from retrieval. A refresh replaces the complete cached set atomically, so keys
removed by the provider disappear from the next cache generation.

Scafra loads keys on the first remote JWT request. It refreshes an expired
cache before using any key from it. An expired cache is never used when refresh
fails, discovery fails, the provider returns a non-success status, the response
is too large, or JSON/JWK parsing fails. The request is denied with the existing
generic `401` response.

When a JWT has an unknown `kid` and the cache is still fresh, Scafra performs
one refresh attempt. The new full key set is then used to resolve the key; if
the `kid` is still absent, authentication is denied. Repeated misses have a
30-second refresh cooldown, including failed provider attempts, to bound
requests for attacker-chosen `kid` values. Unknown `kid` requests during the
cooldown are denied without another provider call. Initial-load and expired
cache refreshes also have a 30-second retry cooldown after a failure.

A key already in a fresh cache can validate tokens until the five-minute cache
lifetime expires, even if the provider has become unavailable, has removed that
key, or starts returning malformed documents. This is the rotation overlap
policy. After expiration, Scafra must
successfully retrieve and parse a replacement key set before accepting any
remote-key JWT. If a key disappears or is replaced under the same `kid`, the
change takes effect at the next successful refresh; a removed or replaced key
is never accepted from an expired generation.

Discovery and JWKS requests use a two-second connection timeout and a
five-second total timeout for the discovery-plus-JWKS operation. Each response
is limited to one mebibyte, redirects are rejected, and no provider retry loop
is used. Timeouts, non-success responses, malformed metadata, and malformed
JWKS documents fail closed. Provider response bodies and internal parsing or
network errors are neither logged nor returned to clients. The async refresh
gate coalesces simultaneous cache misses and expiry refreshes; it is scoped to
the router and does not block Tokio worker threads. A waiting request rechecks
the cache after the refresh completes.

For a planned key rotation, publish the new public key before signing tokens
with its `kid`, keep the old key available through the maximum five-minute
cache lifetime and the application's clock-skew window, then remove the old
key. Scafra does not use provider `Cache-Control` headers, implement periodic
background refresh, or enforce an IP-range policy for operator-configured
provider URLs. Only HTTP(S) URLs are accepted. Request timeout and cache policy
are fixed rather than configurable in this API.

## Migration impact and limits

Existing applications with security disabled keep their public-by-default
behavior. Existing applications with security enabled keep the global
protected-by-default behavior and `permit_all` exceptions. Add controller and
route annotations when a narrower policy is needed. If a controller or route
is marked protected, set `security.enabled: true` and configure a bearer token,
Basic credentials, or JWT before building its router.

JWTs that previously failed the `required_scopes` check as authentication
failures now return `403` when their signature and standard claims are valid
but they lack a required scope. Invalid signatures, issuer/audience mismatches,
missing expiration claims, and expired tokens remain `401`.

Scafra does not fetch user roles from an external directory or interpret
provider-specific nested role claims; applications must issue the documented
claim shapes. Scafra does not terminate TLS, so deploy Basic authentication,
shared bearer tokens, and JWTs over HTTPS.

The configuration types retain their existing serde shape and crate-root
imports. Their `Debug` representation now redacts credential values; code that
compared the old debug-formatted output should account for that intentional
diagnostic change.
