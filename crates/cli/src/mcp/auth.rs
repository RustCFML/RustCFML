//! Who may reach an MCP endpoint, and what they may do once there.
//!
//! An MCP server is a remote-control surface for the application, so the
//! defaults are closed: only this machine may connect, and `secured` handlers
//! stay unreachable until tokens exist to authenticate against. Everything
//! here is pure so the matching rules are testable without a socket.

use std::net::IpAddr;

use axum::http::HeaderMap;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_config::schema::{McpCfg, McpToken};

/// An authorized caller: who they are, and which tools they may use.
#[derive(Clone, Debug, Default)]
pub(crate) struct Caller {
    /// Identity for the `secured` gate, in the shape it expects
    /// (`{ authenticated, roles }`) — the same contract a WebSocket handler's
    /// `socket.data` uses. `None` for an unauthenticated caller.
    pub(crate) identity: Option<CfmlValue>,
    included: Vec<String>,
    excluded: Vec<String>,
    /// Set when the caller's scopes map to no tools at all. Distinct from an
    /// empty `included`, which means "no allow-list" (everything).
    none_included: bool,
    /// The raw `Authorization` header, for `mcp().authorization()`.
    pub(crate) authorization: Option<String>,
    /// `"http"` or `"stdio"`, for `mcp().transport()`.
    pub(crate) transport: &'static str,
}

/// What [`decide`] concluded from the configuration alone.
#[derive(Debug)]
pub(crate) enum Decision {
    /// Settled: the caller is this (a static token, or nobody needed one).
    Done(Caller),
    /// A bearer token no static entry knows, and an `authenticate` hook is
    /// configured: the transport must ask the application.
    Resolve(String),
}

/// Why a request was turned away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Denied {
    /// MCP is switched off for this application.
    Disabled,
    /// The caller's address is not allowed.
    Address,
    /// A token was required and was missing or wrong.
    Token,
    /// A browser origin that is not allowed.
    Origin,
}

impl Denied {
    /// Deliberately terse. A probe should not learn whether it guessed a
    /// valid token, or whether the endpoint exists but rejected its address.
    pub(crate) fn message(self) -> &'static str {
        match self {
            Denied::Disabled => "MCP is not enabled on this server",
            Denied::Address => "Not authorized",
            Denied::Token => "Not authorized",
            Denied::Origin => "Origin not allowed",
        }
    }
}

impl Caller {
    /// A caller on the stdio transport: a subprocess the user launched
    /// themselves. Authenticated by construction — but its roles come from
    /// configuration, so `secured="admin"` is still granted deliberately
    /// rather than by virtue of having run the binary.
    pub(crate) fn stdio(cfg: &McpCfg) -> Self {
        Self {
            identity: Some(identity(&cfg.stdio_roles)),
            included: cfg.included_tools.clone(),
            excluded: cfg.excluded_tools.clone(),
            transport: "stdio",
            ..Default::default()
        }
    }

    /// An unauthenticated caller under the global filters.
    fn anonymous(cfg: &McpCfg) -> Self {
        Self {
            identity: None,
            included: cfg.included_tools.clone(),
            excluded: cfg.excluded_tools.clone(),
            transport: "http",
            ..Default::default()
        }
    }

    /// May this caller use the named tool?
    pub(crate) fn may_call(&self, tool: &str) -> bool {
        if self.none_included {
            return false;
        }
        let included = if self.included.is_empty() {
            true
        } else {
            self.included.iter().any(|p| glob_match(p, tool))
        };
        // Exclusions are applied after inclusions, so a broad `*` can be
        // narrowed by naming the exceptions.
        included && !self.excluded.iter().any(|p| glob_match(p, tool))
    }
}

fn identity(roles: &[String]) -> CfmlValue {
    let mut map = ValueMap::default();
    map.insert("authenticated".to_string(), CfmlValue::Bool(true));
    map.insert(
        "roles".to_string(),
        CfmlValue::array(roles.iter().map(|r| CfmlValue::string(r.clone())).collect()),
    );
    CfmlValue::strukt(map)
}

/// Decide whether an HTTP request may proceed, and as whom, when no
/// `authenticate` hook can be consulted. A token that only the hook could
/// resolve is a refusal here.
#[cfg(test)]
pub(crate) fn authorize(
    cfg: &McpCfg,
    peer: IpAddr,
    headers: &HeaderMap,
) -> Result<Caller, Denied> {
    match decide(cfg, peer, headers)? {
        Decision::Done(caller) => Ok(caller),
        Decision::Resolve(_) => Err(Denied::Token),
    }
}

/// Everything that can be decided from configuration: is MCP on, is the
/// address and origin allowed, and does the bearer token match a static
/// entry. A token no static entry knows is handed back for the
/// `authenticate` hook when one is configured.
pub(crate) fn decide(
    cfg: &McpCfg,
    peer: IpAddr,
    headers: &HeaderMap,
) -> Result<Decision, Denied> {
    if !cfg.enabled {
        return Err(Denied::Disabled);
    }
    if !ip_allowed(&cfg.allowed_ips, peer) {
        return Err(Denied::Address);
    }
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        if !origin_allowed(cfg, origin) {
            return Err(Denied::Origin);
        }
    }
    let authorization =
        headers.get("authorization").and_then(|v| v.to_str().ok()).map(String::from);

    // No tokens and no hook: the endpoint is open to whoever the address rules
    // let through, and nobody is authenticated — so `secured` handlers stay
    // unreachable, which is the honest outcome of having configured no way to
    // identify anyone.
    if cfg.auth_tokens.is_empty() && !has_hook(cfg) {
        return Ok(Decision::Done(Caller { authorization, ..Caller::anonymous(cfg) }));
    }

    let presented = bearer(headers).ok_or(Denied::Token)?;
    match resolve_static(cfg, &presented) {
        Some(mut caller) => {
            caller.authorization = authorization;
            Ok(Decision::Done(caller))
        }
        None if has_hook(cfg) => Ok(Decision::Resolve(presented)),
        None => Err(Denied::Token),
    }
}

/// Is an `authenticate` hook configured?
pub(crate) fn has_hook(cfg: &McpCfg) -> bool {
    !cfg.authenticate.trim().is_empty()
}

/// Match a credential against the static `authToken` list.
pub(crate) fn resolve_static(cfg: &McpCfg, presented: &str) -> Option<Caller> {
    cfg.auth_tokens
        .iter()
        .find(|t| constant_time_eq(&t.token, presented))
        .map(|t| caller_for(cfg, t))
}

/// Turn what an `authenticate` hook returned into a caller, or `None` for a
/// refusal.
///
/// Null, anything that is not a struct, and a struct that says
/// `authenticated = false` all refuse. Otherwise the struct *is* the identity:
/// `authenticated` and `roles` are normalised for the `secured` gate, and every
/// other key travels with it to `mcp().identity()` untouched. Tool filters
/// come from, in order: `includedTools`/`excludedTools` on the struct (which
/// replace the global ones, as a token object's do); the `scopes` mapping, when
/// the identity carries scopes; the global filters.
pub(crate) fn caller_from_identity(cfg: &McpCfg, value: CfmlValue) -> Option<Caller> {
    let CfmlValue::Struct(id) = value else { return None };
    if let Some(flag) = id.get_ci("authenticated") {
        if !flag.is_true() {
            return None;
        }
    }
    // Work on a copy so normalising cannot reach back into whatever the
    // application cached the struct in.
    let CfmlValue::Struct(id) = CfmlValue::Struct(id).deep_copy() else { return None };
    id.insert("authenticated".to_string(), CfmlValue::Bool(true));
    let roles = string_list(id.get_ci("roles"));
    id.insert(
        "roles".to_string(),
        CfmlValue::array(roles.into_iter().map(CfmlValue::string).collect()),
    );

    let own_included = string_list(id.get_ci("includedTools"));
    let own_excluded = string_list(id.get_ci("excludedTools"));
    let scopes = string_list(id.get_ci("scopes").or_else(|| id.get_ci("scope")));

    let mut caller = Caller {
        included: cfg.included_tools.clone(),
        excluded: cfg.excluded_tools.clone(),
        transport: "http",
        ..Default::default()
    };
    if !cfg.scopes.is_empty() && !scopes.is_empty() {
        // A scope narrows what is visible: the union of what the caller's
        // scopes include. The global exclusions still apply on top.
        let mut included = Vec::new();
        let mut excluded = cfg.excluded_tools.clone();
        for scope in &scopes {
            if let Some((_, grant)) =
                cfg.scopes.iter().find(|(name, _)| name.eq_ignore_ascii_case(scope))
            {
                included.extend(grant.included_tools.iter().cloned());
                excluded.extend(grant.excluded_tools.iter().cloned());
            }
        }
        caller.none_included = included.is_empty();
        caller.included = included;
        caller.excluded = excluded;
    }
    if !own_included.is_empty() {
        caller.included = own_included;
        caller.none_included = false;
    }
    if !own_excluded.is_empty() {
        caller.excluded = own_excluded;
    }
    caller.identity = Some(CfmlValue::Struct(id));
    Some(caller)
}

/// An array of strings, or a comma/space separated list — the two shapes an
/// application is likely to hand back (`roles = ["a","b"]`, `scope = "read write"`).
fn string_list(value: Option<CfmlValue>) -> Vec<String> {
    match value {
        Some(CfmlValue::Array(arr)) => arr
            .iter()
            .map(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(CfmlValue::Null) | None => Vec::new(),
        Some(other) => other
            .as_string()
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
    }
}

/// The `WWW-Authenticate` value for a 401. With OAuth configured it points
/// the client at the protected-resource metadata (RFC 9728 §5.1), which is how
/// a client discovers which authorization server to go to; without it, a bare
/// `Bearer`.
pub(crate) fn challenge(cfg: &McpCfg, headers: &HeaderMap, server: &str) -> String {
    if !cfg.oauth.enabled() {
        return "Bearer".to_string();
    }
    let url = metadata_url(cfg, headers, server);
    format!("Bearer resource_metadata=\"{url}\"")
}

/// Where this server's protected-resource metadata lives: the well-known path
/// inserted between the resource's origin and its path (RFC 9728 §3.1).
pub(crate) fn metadata_url(cfg: &McpCfg, headers: &HeaderMap, server: &str) -> String {
    let resource = resource_for(cfg, headers, &format!("/mcp/{server}"));
    let (origin, path) = split_origin(&resource);
    format!("{origin}/.well-known/oauth-protected-resource{path}")
}

/// The resource identifier: as configured, or derived from the request.
pub(crate) fn resource_for(cfg: &McpCfg, headers: &HeaderMap, path: &str) -> String {
    let configured = cfg.oauth.resource.trim();
    if !configured.is_empty() {
        return configured.trim_end_matches('/').to_string();
    }
    format!("{}{}", request_origin(headers), path)
}

/// `scheme://host` as the client addressed us. Behind a TLS-terminating proxy
/// the scheme comes from `X-Forwarded-Proto`.
pub(crate) fn request_origin(headers: &HeaderMap) -> String {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    let scheme = get("x-forwarded-proto")
        .and_then(|p| p.split(',').next())
        .map(str::trim)
        .filter(|p| *p == "http" || *p == "https")
        .unwrap_or("http");
    let host = get("x-forwarded-host")
        .and_then(|h| h.split(',').next())
        .map(str::trim)
        .or_else(|| get("host"))
        .filter(|h| !h.is_empty() && !h.contains(['"', ' ', '/']))
        .unwrap_or("localhost");
    format!("{scheme}://{host}")
}

fn split_origin(url: &str) -> (&str, &str) {
    let after_scheme = url.find("://").map(|i| i + 3).unwrap_or(0);
    match url[after_scheme..].find('/') {
        Some(i) => url.split_at(after_scheme + i),
        None => (url, ""),
    }
}

fn caller_for(cfg: &McpCfg, token: &McpToken) -> Caller {
    // A token's own filters replace the global ones when it declares any, so
    // a restricted token can be narrower than the default without having to
    // restate the whole policy.
    let included = if token.included_tools.is_empty() {
        cfg.included_tools.clone()
    } else {
        token.included_tools.clone()
    };
    let excluded = if token.excluded_tools.is_empty() {
        cfg.excluded_tools.clone()
    } else {
        token.excluded_tools.clone()
    };
    Caller {
        identity: Some(identity(&token.roles)),
        included,
        excluded,
        transport: "http",
        ..Default::default()
    }
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("authorization")?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Compare without an early return on the first differing byte, so the time
/// taken does not reveal how much of a guessed token was correct.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Is the caller's address allowed? An empty list means "anywhere", which is
/// only sensible behind a proxy that authenticates on your behalf.
pub(crate) fn ip_allowed(allowed: &[String], peer: IpAddr) -> bool {
    if allowed.is_empty() {
        return true;
    }
    allowed.iter().any(|rule| match rule.split_once('/') {
        Some((base, bits)) => match (base.parse::<IpAddr>(), bits.parse::<u8>()) {
            (Ok(base), Ok(bits)) => in_cidr(base, bits, peer),
            _ => false,
        },
        None => rule.parse::<IpAddr>().map(|a| same_address(a, peer)).unwrap_or(false),
    })
}

/// Treat `::ffff:127.0.0.1` as `127.0.0.1`: a dual-stack listener reports
/// IPv4 clients in that mapped form, so an `allowedIPs` of `127.0.0.1` would
/// otherwise reject the loopback it was written for.
fn normalize(addr: IpAddr) -> IpAddr {
    match addr {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(addr),
        v4 => v4,
    }
}

fn same_address(a: IpAddr, b: IpAddr) -> bool {
    normalize(a) == normalize(b)
}

fn in_cidr(base: IpAddr, bits: u8, peer: IpAddr) -> bool {
    match (normalize(base), normalize(peer)) {
        (IpAddr::V4(base), IpAddr::V4(peer)) => {
            if bits > 32 {
                return false;
            }
            if bits == 0 {
                return true;
            }
            let mask = u32::MAX << (32 - bits);
            u32::from(base) & mask == u32::from(peer) & mask
        }
        (IpAddr::V6(base), IpAddr::V6(peer)) => {
            if bits > 128 {
                return false;
            }
            if bits == 0 {
                return true;
            }
            let mask = u128::MAX << (128 - bits);
            u128::from(base) & mask == u128::from(peer) & mask
        }
        // Mixing families never matches; it is a configuration mistake rather
        // than something to guess at.
        _ => false,
    }
}

/// Localhost is always allowed: a page served from this machine is not the
/// DNS-rebinding threat the origin check exists for. Anything else must be
/// listed in `corsAllowedOrigins`, where `*` means any.
pub(crate) fn origin_allowed(cfg: &McpCfg, origin: &str) -> bool {
    if origin_is_local(origin) {
        return true;
    }
    cfg.cors_allowed_origins
        .iter()
        .any(|allowed| allowed == "*" || glob_match(allowed, origin))
}

fn origin_is_local(origin: &str) -> bool {
    let authority = origin.split("//").nth(1).unwrap_or(origin);
    let host = authority.rsplit_once(':').map(|(h, _)| h).unwrap_or(authority);
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

/// Shell-style glob with `*` (any run, including none) and `?` (one
/// character), matched case-insensitively because CFML identifiers are.
pub(crate) fn glob_match(pattern: &str, value: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let v: Vec<char> = value.to_lowercase().chars().collect();
    let (mut pi, mut vi) = (0usize, 0usize);
    // Position of the last `*` and where in the value it had matched to, so a
    // failed branch can resume by letting that `*` swallow one more character.
    let (mut star, mut resume) = (None, 0usize);
    while vi < v.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == v[vi]) {
            pi += 1;
            vi += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            resume = vi;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            resume += 1;
            vi = resume;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> McpCfg {
        McpCfg::default()
    }

    fn with_token(token: &str, roles: &[&str]) -> McpCfg {
        let mut c = cfg();
        c.auth_tokens = vec![McpToken {
            token: token.to_string(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
            ..Default::default()
        }];
        c
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    fn local() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }

    #[test]
    fn the_default_configuration_is_localhost_only() {
        let cfg = cfg();
        assert!(ip_allowed(&cfg.allowed_ips, local()));
        assert!(ip_allowed(&cfg.allowed_ips, "::1".parse().unwrap()));
        assert!(!ip_allowed(&cfg.allowed_ips, "10.0.0.5".parse().unwrap()));
        assert!(!ip_allowed(&cfg.allowed_ips, "8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn an_ipv4_mapped_address_matches_its_ipv4_rule() {
        // A dual-stack listener reports IPv4 clients as ::ffff:127.0.0.1, so
        // without this an `allowedIPs` of 127.0.0.1 rejects the loopback it
        // was written for.
        let rules = vec!["127.0.0.1".to_string()];
        assert!(ip_allowed(&rules, "::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn cidr_ranges_match_by_prefix() {
        let rules = vec!["10.0.0.0/8".to_string(), "192.168.1.0/24".to_string()];
        assert!(ip_allowed(&rules, "10.4.5.6".parse().unwrap()));
        assert!(ip_allowed(&rules, "192.168.1.77".parse().unwrap()));
        assert!(!ip_allowed(&rules, "192.168.2.77".parse().unwrap()));
        assert!(!ip_allowed(&rules, "11.0.0.1".parse().unwrap()));

        assert!(ip_allowed(&vec!["0.0.0.0/0".to_string()], "8.8.8.8".parse().unwrap()));
        assert!(ip_allowed(&vec!["::/0".to_string()], "2001:db8::1".parse().unwrap()));
        // A v4 rule must not silently admit a v6 caller.
        assert!(!ip_allowed(&vec!["10.0.0.0/8".to_string()], "2001:db8::1".parse().unwrap()));
        // Nonsense rules deny rather than crash or admit.
        assert!(!ip_allowed(&vec!["not-an-ip".to_string()], local()));
        assert!(!ip_allowed(&vec!["10.0.0.0/99".to_string()], "10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn an_empty_allow_list_permits_any_address() {
        assert!(ip_allowed(&[], "8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn a_disabled_server_refuses_everything() {
        let mut cfg = cfg();
        cfg.enabled = false;
        assert_eq!(authorize(&cfg, local(), &HeaderMap::new()).unwrap_err(), Denied::Disabled);
    }

    #[test]
    fn with_no_tokens_configured_nobody_is_authenticated() {
        // Which is why `secured` handlers stay unreachable until tokens exist:
        // there is no way to identify anyone.
        let caller = authorize(&cfg(), local(), &HeaderMap::new()).expect("allowed");
        assert!(caller.identity.is_none());
    }

    #[test]
    fn a_configured_token_authenticates_and_carries_its_roles() {
        let cfg = with_token("s3cret", &["admin", "ops"]);
        assert_eq!(
            authorize(&cfg, local(), &HeaderMap::new()).unwrap_err(),
            Denied::Token,
            "a token is required once any is configured"
        );
        assert_eq!(
            authorize(&cfg, local(), &headers(&[("authorization", "Bearer wrong")])).unwrap_err(),
            Denied::Token
        );
        // Not a bearer scheme at all.
        assert_eq!(
            authorize(&cfg, local(), &headers(&[("authorization", "Basic s3cret")])).unwrap_err(),
            Denied::Token
        );

        let caller = authorize(&cfg, local(), &headers(&[("authorization", "Bearer s3cret")]))
            .expect("authorized");
        let id = caller.identity.expect("identity");
        let id = id.as_cfml_struct().expect("struct");
        assert!(matches!(id.get_ci("authenticated"), Some(CfmlValue::Bool(true))));
        let CfmlValue::Array(roles) = id.get_ci("roles").expect("roles") else {
            panic!("roles should be an array")
        };
        assert_eq!(roles.len(), 2);
    }

    #[test]
    fn the_bearer_scheme_is_matched_case_insensitively() {
        let cfg = with_token("t", &[]);
        assert!(authorize(&cfg, local(), &headers(&[("authorization", "bearer t")])).is_ok());
        assert!(authorize(&cfg, local(), &headers(&[("authorization", "BEARER t")])).is_ok());
    }

    #[test]
    fn denial_messages_do_not_say_which_check_failed() {
        // A probe should not be able to tell a wrong token from a blocked
        // address, or learn that the endpoint exists at all.
        assert_eq!(Denied::Token.message(), Denied::Address.message());
    }

    #[test]
    fn tool_filters_include_then_exclude() {
        let mut cfg = cfg();
        cfg.included_tools = vec!["*".into()];
        cfg.excluded_tools = vec!["delete_*".into()];
        let caller = authorize(&cfg, local(), &HeaderMap::new()).expect("allowed");
        assert!(caller.may_call("search_docs"));
        assert!(!caller.may_call("delete_everything"));

        cfg.included_tools = vec!["read_*".into(), "search".into()];
        cfg.excluded_tools = vec![];
        let caller = authorize(&cfg, local(), &HeaderMap::new()).expect("allowed");
        assert!(caller.may_call("read_file"));
        assert!(caller.may_call("search"));
        assert!(!caller.may_call("write_file"));
    }

    #[test]
    fn a_tokens_own_filters_replace_the_global_ones() {
        let mut cfg = with_token("ro", &["readonly"]);
        cfg.included_tools = vec!["*".into()];
        cfg.auth_tokens[0].included_tools = vec!["get_*".into()];

        let caller = authorize(&cfg, local(), &headers(&[("authorization", "Bearer ro")]))
            .expect("authorized");
        assert!(caller.may_call("get_status"));
        assert!(!caller.may_call("restart_server"), "the token is narrower than the default");
    }

    #[test]
    fn stdio_callers_are_authenticated_but_hold_only_configured_roles() {
        let caller = Caller::stdio(&cfg());
        let id = caller.identity.expect("authenticated");
        let id = id.as_cfml_struct().expect("struct");
        assert!(matches!(id.get_ci("authenticated"), Some(CfmlValue::Bool(true))));
        let CfmlValue::Array(roles) = id.get_ci("roles").expect("roles") else {
            panic!("array")
        };
        assert_eq!(roles.len(), 0, "running the binary must not confer a named role");

        let mut cfg = cfg();
        cfg.stdio_roles = vec!["admin".into()];
        let caller = Caller::stdio(&cfg);
        let id = caller.identity.expect("authenticated");
        let CfmlValue::Array(roles) = id.as_cfml_struct().unwrap().get_ci("roles").unwrap() else {
            panic!("array")
        };
        assert_eq!(roles.len(), 1);
    }

    #[test]
    fn origins_are_localhost_plus_whatever_is_configured() {
        let mut cfg = cfg();
        assert!(origin_allowed(&cfg, "http://localhost:8500"));
        assert!(!origin_allowed(&cfg, "https://app.example.com"));

        cfg.cors_allowed_origins = vec!["https://*.example.com".into()];
        assert!(origin_allowed(&cfg, "https://app.example.com"));
        assert!(!origin_allowed(&cfg, "https://evil.test"));

        cfg.cors_allowed_origins = vec!["*".into()];
        assert!(origin_allowed(&cfg, "https://anything.test"));
    }

    #[test]
    fn a_blocked_origin_is_refused_by_authorize() {
        let cfg = cfg();
        assert_eq!(
            authorize(&cfg, local(), &headers(&[("origin", "https://evil.test")])).unwrap_err(),
            Denied::Origin
        );
        assert!(authorize(&cfg, local(), &headers(&[("origin", "http://localhost:1")])).is_ok());
    }

    fn with_hook() -> McpCfg {
        let mut c = with_token("static", &["admin"]);
        c.authenticate = "auth.McpAuth".into();
        c
    }

    fn ident(pairs: &[(&str, CfmlValue)]) -> CfmlValue {
        let mut m = ValueMap::default();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        CfmlValue::strukt(m)
    }

    fn strs(items: &[&str]) -> CfmlValue {
        CfmlValue::array(items.iter().map(|s| CfmlValue::string(s.to_string())).collect())
    }

    #[test]
    fn a_static_token_wins_before_the_hook_is_asked() {
        let cfg = with_hook();
        let h = headers(&[("authorization", "Bearer static")]);
        assert!(matches!(decide(&cfg, local(), &h), Ok(Decision::Done(_))));
        let h = headers(&[("authorization", "Bearer user-42")]);
        match decide(&cfg, local(), &h) {
            Ok(Decision::Resolve(t)) => assert_eq!(t, "user-42"),
            other => panic!("expected the hook to be asked, got {other:?}"),
        }
        // No token at all is still a 401, hook or not.
        assert_eq!(decide(&cfg, local(), &HeaderMap::new()).unwrap_err(), Denied::Token);
    }

    #[test]
    fn a_hook_alone_turns_on_authentication() {
        let mut cfg = cfg();
        cfg.authenticate = "auth.McpAuth".into();
        assert_eq!(decide(&cfg, local(), &HeaderMap::new()).unwrap_err(), Denied::Token);
        assert!(matches!(
            decide(&cfg, local(), &headers(&[("authorization", "Bearer x")])),
            Ok(Decision::Resolve(_))
        ));
    }

    #[test]
    fn what_the_hook_returns_becomes_the_identity() {
        let cfg = with_hook();
        assert!(caller_from_identity(&cfg, CfmlValue::Null).is_none(), "null refuses");
        assert!(caller_from_identity(&cfg, CfmlValue::string("yes")).is_none());
        assert!(
            caller_from_identity(&cfg, ident(&[("authenticated", CfmlValue::Bool(false))]))
                .is_none(),
            "an explicit authenticated=false refuses"
        );

        let caller = caller_from_identity(
            &cfg,
            ident(&[
                ("roles", CfmlValue::string("member, editor")),
                ("principalId", CfmlValue::Int(42)),
            ]),
        )
        .expect("accepted");
        let id = caller.identity.expect("identity");
        let id = id.as_cfml_struct().unwrap();
        assert!(matches!(id.get_ci("authenticated"), Some(CfmlValue::Bool(true))));
        let CfmlValue::Array(roles) = id.get_ci("roles").unwrap() else { panic!("array") };
        assert_eq!(roles.len(), 2, "a comma list is normalised to an array");
        assert!(matches!(id.get_ci("principalId"), Some(CfmlValue::Int(42))), "extra keys travel");
    }

    #[test]
    fn the_hook_may_narrow_the_tool_filters() {
        let cfg = with_hook();
        let caller =
            caller_from_identity(&cfg, ident(&[("includedTools", strs(&["get_*"]))])).unwrap();
        assert!(caller.may_call("get_status"));
        assert!(!caller.may_call("restart"));
    }

    #[test]
    fn scopes_map_to_tools() {
        let mut cfg = with_hook();
        cfg.excluded_tools = vec!["get_secret".into()];
        cfg.scopes.insert(
            "read".into(),
            cfml_config::schema::McpScope { included_tools: vec!["get_*".into()], ..Default::default() },
        );
        cfg.scopes.insert(
            "write".into(),
            cfml_config::schema::McpScope { included_tools: vec!["set_*".into()], ..Default::default() },
        );

        let reader =
            caller_from_identity(&cfg, ident(&[("scope", CfmlValue::string("read"))]))
                .unwrap();
        assert!(reader.may_call("get_status"));
        assert!(!reader.may_call("set_status"));
        assert!(!reader.may_call("get_secret"), "global exclusions still apply");

        let both = caller_from_identity(&cfg, ident(&[("scopes", strs(&["read", "write"]))]))
            .unwrap();
        assert!(both.may_call("get_status") && both.may_call("set_status"));

        // A scope the mapping does not know grants nothing — not everything.
        let stranger =
            caller_from_identity(&cfg, ident(&[("scopes", strs(&["admin"]))])).unwrap();
        assert!(!stranger.may_call("get_status"));

        // No scopes on the identity: the global filters, as before.
        let plain = caller_from_identity(&cfg, ident(&[])).unwrap();
        assert!(plain.may_call("set_status"));
    }

    #[test]
    fn the_challenge_points_at_the_metadata_only_when_oauth_is_configured() {
        let mut cfg = cfg();
        let h = headers(&[("host", "api.example.com")]);
        assert_eq!(challenge(&cfg, &h, "docs"), "Bearer");

        cfg.oauth.authorization_servers = vec!["https://auth.example.com".into()];
        assert_eq!(
            challenge(&cfg, &h, "docs"),
            "Bearer resource_metadata=\"http://api.example.com/.well-known/oauth-protected-resource/mcp/docs\""
        );
        let h = headers(&[("host", "internal:8500"), ("x-forwarded-proto", "https"),
                          ("x-forwarded-host", "api.example.com")]);
        assert!(challenge(&cfg, &h, "docs")
            .contains("https://api.example.com/.well-known/oauth-protected-resource/mcp/docs"));

        cfg.oauth.resource = "https://mcp.example.com/tools/".into();
        assert!(challenge(&cfg, &h, "docs")
            .contains("\"https://mcp.example.com/.well-known/oauth-protected-resource/tools\""));
    }

    #[test]
    fn globs_handle_stars_questions_and_case() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("get_*", "get_status"));
        assert!(glob_match("*_get_*", "jvm_get_memory"));
        assert!(glob_match("GET_?", "get_x"), "matching is case-insensitive");
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("get_*", "set_status"));
        assert!(!glob_match("get_?", "get_xy"));
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("a*b*c", "axxbyy"));
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
    }
}
