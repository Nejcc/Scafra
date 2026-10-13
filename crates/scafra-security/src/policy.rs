//! Public route policy metadata and private route matching and resolution.

use axum::http::Method;

/// Whether a controller or route inherits the application-wide policy,
/// explicitly allows anonymous access, or requires authentication.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AuthorizationMode {
    #[default]
    Inherit,
    Public,
    Protected,
}

/// Authorization requirements declared on a controller or route.
///
/// Every role group must match at least one role. Every listed scope is
/// required. An empty policy with [`AuthorizationMode::Inherit`] uses the
/// application-wide security setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuthorizationPolicy {
    pub mode: AuthorizationMode,
    pub role_groups: &'static [&'static [&'static str]],
    pub required_scopes: &'static [&'static str],
}

impl AuthorizationPolicy {
    /// Inherit the application-wide security setting.
    pub const fn inherit() -> Self {
        Self {
            mode: AuthorizationMode::Inherit,
            role_groups: &[],
            required_scopes: &[],
        }
    }

    /// Explicitly allow anonymous access.
    pub const fn public() -> Self {
        Self {
            mode: AuthorizationMode::Public,
            role_groups: &[],
            required_scopes: &[],
        }
    }

    /// Require authentication.
    pub const fn protected() -> Self {
        Self {
            mode: AuthorizationMode::Protected,
            role_groups: &[],
            required_scopes: &[],
        }
    }

    /// Returns whether this policy explicitly requires authentication.
    pub fn is_protected(self) -> bool {
        self.mode == AuthorizationMode::Protected
    }
}

/// Authorization metadata for one macro-generated controller route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteAuthorizationMetadata {
    pub controller: &'static str,
    pub method: &'static str,
    pub prefix: &'static str,
    pub path: &'static str,
    pub controller_policy: AuthorizationPolicy,
    pub route_policy: AuthorizationPolicy,
}

/// Link-time route policy metadata emitted by the `#[routes]` macro.
pub struct ControllerAuthorizationRegistration {
    pub controller: &'static str,
    pub routes: &'static [RouteAuthorizationMetadata],
}

inventory::collect!(ControllerAuthorizationRegistration);

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ResolvedPolicy {
    pub(super) is_public: bool,
    pub(super) is_protected: bool,
    pub(super) controller_role_groups: &'static [&'static [&'static str]],
    pub(super) route_role_groups: &'static [&'static [&'static str]],
    pub(super) controller_scopes: &'static [&'static str],
    pub(super) route_scopes: &'static [&'static str],
}

impl ResolvedPolicy {
    pub(super) fn is_public(self) -> bool {
        self.is_public
    }

    pub(super) fn is_protected(self) -> bool {
        self.is_protected
    }
}

pub(super) struct RoutePolicyMatcher {
    policies: Vec<PreparedRoutePolicy>,
    has_protected_routes: bool,
}

struct PreparedRoutePolicy {
    metadata: RouteAuthorizationMetadata,
    path: String,
    specificity: Vec<u8>,
    resolved: ResolvedPolicy,
    protection_rank: (usize, usize),
}

impl RoutePolicyMatcher {
    pub(super) fn new(policies: Vec<RouteAuthorizationMetadata>) -> Self {
        let policies = policies
            .into_iter()
            .filter(|metadata| !is_inert(metadata))
            .map(|metadata| {
                let path = join_paths(metadata.prefix, metadata.path);
                let specificity = path_specificity(&path);
                let resolved = resolve_policy(&metadata);
                let protection_rank = (
                    resolved.controller_role_groups.len() + resolved.route_role_groups.len(),
                    resolved.controller_scopes.len() + resolved.route_scopes.len(),
                );
                PreparedRoutePolicy {
                    metadata,
                    path,
                    specificity,
                    resolved,
                    protection_rank,
                }
            })
            .collect::<Vec<_>>();
        let has_protected_routes = policies.iter().any(|policy| policy.resolved.is_protected);

        Self {
            policies,
            has_protected_routes,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    pub(super) fn has_protected_routes(&self) -> bool {
        self.has_protected_routes
    }
}

pub(super) fn matching_policy(
    matcher: &RoutePolicyMatcher,
    method: &Method,
    path: &str,
) -> Option<ResolvedPolicy> {
    let method = if method == Method::HEAD {
        "GET"
    } else {
        method.as_str()
    };

    let mut method_match: Option<&PreparedRoutePolicy> = None;
    let mut protected_fallback: Option<(ResolvedPolicy, (usize, usize))> = None;
    for candidate in &matcher.policies {
        if !route_path_matches(&candidate.path, path) {
            continue;
        }

        if candidate.metadata.method.eq_ignore_ascii_case(method)
            && method_match.is_none_or(|current| candidate.specificity >= current.specificity)
        {
            // Axum prefers static path segments over parameters. Comparing
            // precomputed specificity preserves that precedence without
            // allocating a vector for each matching request.
            method_match = Some(candidate);
        }

        // A method with no matching route must not bypass a protected path
        // and then reach a different fallback. Public access stays
        // method-specific. Keep the same role/scope precedence as before.
        let resolved = candidate.resolved;
        if resolved.is_protected {
            let rank = candidate.protection_rank;
            if protected_fallback.is_none_or(|(_, current_rank)| rank >= current_rank) {
                protected_fallback = Some((resolved, rank));
            }
        }
    }

    method_match
        .map(|policy| policy.resolved)
        .or_else(|| protected_fallback.map(|(policy, _)| policy))
}

fn is_inert(metadata: &RouteAuthorizationMetadata) -> bool {
    let controller = metadata.controller_policy;
    let route = metadata.route_policy;
    controller.mode == AuthorizationMode::Inherit
        && route.mode == AuthorizationMode::Inherit
        && controller.role_groups.is_empty()
        && controller.required_scopes.is_empty()
        && route.role_groups.is_empty()
        && route.required_scopes.is_empty()
}

fn resolve_policy(metadata: &RouteAuthorizationMetadata) -> ResolvedPolicy {
    let controller = metadata.controller_policy;
    let route = metadata.route_policy;
    if route.mode == AuthorizationMode::Public {
        return ResolvedPolicy {
            is_public: true,
            ..ResolvedPolicy::default()
        };
    }
    if route.mode == AuthorizationMode::Inherit && controller.mode == AuthorizationMode::Public {
        return ResolvedPolicy {
            is_public: true,
            ..ResolvedPolicy::default()
        };
    }

    let is_protected = route.mode == AuthorizationMode::Protected
        || controller.mode == AuthorizationMode::Protected;
    ResolvedPolicy {
        is_public: false,
        is_protected,
        controller_role_groups: if is_protected {
            controller.role_groups
        } else {
            &[]
        },
        route_role_groups: if is_protected { route.role_groups } else { &[] },
        controller_scopes: if is_protected {
            controller.required_scopes
        } else {
            &[]
        },
        route_scopes: if is_protected {
            route.required_scopes
        } else {
            &[]
        },
    }
}

fn route_path_matches(pattern: &str, request_path: &str) -> bool {
    // Axum distinguishes `/path` from `/path/`. Keep the empty final segment
    // instead of normalizing it away so a public route cannot grant access to
    // a different route (or a fallback) with a trailing slash.
    let mut request_segments = path_segments(request_path);
    for segment in path_segments(pattern) {
        if let Some(rest) = segment
            .strip_prefix("{*")
            .and_then(|value| value.strip_suffix('}'))
        {
            let Some(first_remaining) = request_segments.next() else {
                return false;
            };
            return !rest.is_empty()
                && (!first_remaining.is_empty() || request_segments.next().is_some());
        }
        let Some(request_segment) = request_segments.next() else {
            return false;
        };
        if segment.starts_with('{') {
            if request_segment.is_empty() {
                return false;
            }
        } else if segment != request_segment {
            return false;
        }
    }
    request_segments.next().is_none()
}

fn path_segments(path: &str) -> impl Iterator<Item = &str> {
    path.strip_prefix('/').unwrap_or(path).split('/')
}

fn path_specificity(path: &str) -> Vec<u8> {
    path_segments(path)
        .map(|segment| {
            if segment.starts_with("{*") {
                0
            } else if segment.starts_with('{') {
                1
            } else {
                2
            }
        })
        .collect()
}

fn join_paths(prefix: &str, path: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    match (prefix.is_empty(), path.is_empty()) {
        (true, true) => "/".to_owned(),
        (true, false) => format!("/{path}"),
        (false, true) => prefix.to_owned(),
        (false, false) => format!("{prefix}/{path}"),
    }
}
