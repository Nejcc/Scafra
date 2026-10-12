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

pub(super) fn matching_policy(
    policies: &[RouteAuthorizationMetadata],
    method: &Method,
    path: &str,
) -> Option<ResolvedPolicy> {
    let method = if method == Method::HEAD {
        "GET"
    } else {
        method.as_str()
    };
    let matches = policies
        .iter()
        .filter(|policy| route_path_matches(policy, path))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return None;
    }

    let method_matches = matches
        .iter()
        .copied()
        .filter(|policy| policy.method.eq_ignore_ascii_case(method))
        .collect::<Vec<_>>();
    if !method_matches.is_empty() {
        // Axum prefers static path segments over parameters. Matching the most
        // specific declaration avoids a broad parameter policy overriding a
        // more specific static route.
        return method_matches
            .into_iter()
            .max_by(|left, right| path_specificity(left).cmp(&path_specificity(right)))
            .map(resolve_policy);
    }

    // A method with no matching route must not bypass a protected path and
    // then reach a different fallback. Public access stays method-specific.
    matches
        .into_iter()
        .map(resolve_policy)
        .filter(|policy| policy.is_protected)
        .max_by_key(|policy| {
            (
                policy.controller_role_groups.len() + policy.route_role_groups.len(),
                policy.controller_scopes.len() + policy.route_scopes.len(),
            )
        })
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

fn route_path_matches(metadata: &RouteAuthorizationMetadata, request_path: &str) -> bool {
    let pattern = join_paths(metadata.prefix, metadata.path);
    // Axum distinguishes `/path` from `/path/`. Keep the empty final segment
    // instead of normalizing it away so a public route cannot grant access to
    // a different route (or a fallback) with a trailing slash.
    let pattern_segments = path_segments(&pattern);
    let request_segments = path_segments(request_path);
    let mut request_index = 0;
    for (pattern_index, segment) in pattern_segments.iter().enumerate() {
        if let Some(rest) = segment
            .strip_prefix("{*")
            .and_then(|value| value.strip_suffix('}'))
        {
            let Some(remaining) = request_segments.get(request_index..) else {
                return false;
            };
            return !rest.is_empty()
                && !remaining.is_empty()
                && (remaining.len() > 1 || !remaining[0].is_empty());
        }
        let Some(request_segment) = request_segments.get(request_index) else {
            return false;
        };
        if segment.starts_with('{') {
            if request_segment.is_empty() {
                return false;
            }
        } else if segment != request_segment {
            return false;
        }
        request_index += 1;
        if pattern_index + 1 == pattern_segments.len() && request_index != request_segments.len() {
            return false;
        }
    }
    request_index == request_segments.len()
}

fn path_segments(path: &str) -> Vec<&str> {
    path.strip_prefix('/').unwrap_or(path).split('/').collect()
}

fn path_specificity(metadata: &RouteAuthorizationMetadata) -> Vec<u8> {
    let path = join_paths(metadata.prefix, metadata.path);
    path_segments(&path)
        .into_iter()
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
