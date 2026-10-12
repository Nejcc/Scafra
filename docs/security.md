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
- `jwks` performs issuer discovery, retrieves RSA key sets, and caches them.

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

JWT signature and configured issuer, audience, and expiration are validated
before authorization. Scafra requires an `exp` claim. It reads roles from a
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
Scafra's environment configuration rather than committed files.

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

JWKS is fetched once and cached for the lifetime of the process. Key rotation
and cache refresh are outside this issue and are tracked separately by #40.
Scafra does not fetch user roles from an external directory or interpret
provider-specific nested role claims; applications must issue the documented
claim shapes. Scafra does not terminate TLS, so deploy Basic authentication,
shared bearer tokens, and JWTs over HTTPS.

The configuration types retain their existing serde shape and crate-root
imports. Their `Debug` representation now redacts credential values; code that
compared the old debug-formatted output should account for that intentional
diagnostic change.

Application bearer-token validation continues to reject an exactly empty
string but does not trim whitespace. The credential parser rejects whitespace
inside a bearer credential, so a whitespace-only configured token passes
configuration validation but cannot authenticate a request.
