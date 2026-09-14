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
        }
    }

    /// May this caller use the named tool?
    pub(crate) fn may_call(&self, tool: &str) -> bool {
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

/// Decide whether an HTTP request may proceed, and as whom.
pub(crate) fn authorize(
    cfg: &McpCfg,
    peer: IpAddr,
    headers: &HeaderMap,
) -> Result<Caller, Denied> {
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

    // No tokens configured: the endpoint is open to whoever the address rules
    // let through, and nobody is authenticated — so `secured` handlers stay
    // unreachable, which is the honest outcome of having configured no way to
    // identify anyone.
    if cfg.auth_tokens.is_empty() {
        return Ok(Caller {
            identity: None,
            included: cfg.included_tools.clone(),
            excluded: cfg.excluded_tools.clone(),
        });
    }

    let presented = bearer(headers).ok_or(Denied::Token)?;
    let token = cfg
        .auth_tokens
        .iter()
        .find(|t| constant_time_eq(&t.token, &presented))
        .ok_or(Denied::Token)?;
    Ok(caller_for(cfg, token))
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
    Caller { identity: Some(identity(&token.roles)), included, excluded }
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
