//! Shared session-cookie attribute policy.
//!
//! Both runtimes that mint a session `Set-Cookie` — the `--serve` HTTP layer
//! (`crates/cli`) and the Cloudflare Worker fetch handler (`crates/cfml-worker`)
//! — render the cookie through this one builder. Previously each hand-rolled the
//! header string inline and they had drifted apart (the Worker emitted
//! `SameSite=Lax`, the CLI emitted neither `SameSite` nor `Secure`), and neither
//! honoured `this.sessioncookie`. Centralising it kills the drift by construction
//! and lets an application override the attributes.
//!
//! ## The `Secure` default — "secure if the connection is secure"
//!
//! When the app does not set `this.sessioncookie.secure`, `Secure` is emitted iff
//! the request arrived over a secure transport (`conn_is_secure`):
//!
//! * Worker — always HTTPS end-to-end, so `conn_is_secure` is always `true` →
//!   `Secure` on. This also makes `__Secure-`/`__Host-` cookie prefixes possible
//!   later.
//! * CLI — the server is HTTP-only by design and sits behind a TLS-terminating
//!   proxy, so `conn_is_secure` is derived from `X-Forwarded-Proto: https`. A bare
//!   `http://` dev box (LAN IP, custom hostname) therefore gets no `Secure` and
//!   the session survives; a real deployment behind nginx/Caddy gets `Secure`
//!   automatically.
//!
//! This is a deliberate divergence from Lucee, whose spec default is
//! `secure:false` everywhere. The divergence is confined to the *unspecified*
//! case: an explicit `this.sessioncookie.secure = false` is honoured verbatim on
//! both runtimes. See `docs/known-issues.md`.

use crate::dynamic::{CfmlValue, ValueMap};

/// The `SameSite` cookie attribute. `Unset` emits no `SameSite` attribute at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Lax,
    Strict,
    None,
    Unset,
}

impl SameSite {
    /// Parse a `this.sessioncookie.samesite` value. An empty string means
    /// "don't emit the attribute"; anything unrecognised falls back to the
    /// `Lax` default rather than silently dropping it.
    fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "" => SameSite::Unset,
            "strict" => SameSite::Strict,
            "none" => SameSite::None,
            _ => SameSite::Lax,
        }
    }
}

/// Resolved per-application session-cookie attributes, parsed once from
/// `this.sessioncookie` and shared by both runtimes' cookie builders.
#[derive(Debug, Clone)]
pub struct SessionCookiePolicy {
    /// `None` → auto: emit `Secure` iff the connection is secure (the
    /// `conn_is_secure` argument to [`render`](Self::render)). `Some(b)` → the
    /// app set `this.sessioncookie.secure` explicitly and it wins regardless of
    /// transport.
    pub secure: Option<bool>,
    /// Emit `HttpOnly`. Default `true` (matches both prior runtimes).
    pub httponly: bool,
    /// `SameSite` attribute. Default `Lax` on both runtimes (the Worker already
    /// did this; the CLI gains it).
    pub samesite: SameSite,
    /// `Domain` attribute. `None` → omit (host-only cookie).
    pub domain: Option<String>,
    /// `Path` attribute. Default `/`.
    pub path: String,
}

impl Default for SessionCookiePolicy {
    fn default() -> Self {
        SessionCookiePolicy {
            secure: None,
            httponly: true,
            samesite: SameSite::Lax,
            domain: None,
            path: "/".to_string(),
        }
    }
}

impl SessionCookiePolicy {
    /// Parse `this.sessioncookie` out of the lowercased Application.cfc config
    /// map (the same `IndexMap` the other `this.*` session settings are read
    /// from). Absent or non-struct → all defaults. Sub-keys are matched
    /// case-insensitively; unrecognised sub-keys are ignored.
    pub fn from_app_config(config: &ValueMap) -> Self {
        let mut p = SessionCookiePolicy::default();
        if let Some(CfmlValue::Struct(sc)) = config.get("sessioncookie") {
            if let Some(v) = sc.get_ci("secure") {
                p.secure = Some(v.is_true());
            }
            if let Some(v) = sc.get_ci("httponly") {
                p.httponly = v.is_true();
            }
            if let Some(v) = sc.get_ci("samesite") {
                p.samesite = SameSite::parse(&v.as_string());
            }
            if let Some(v) = sc.get_ci("domain") {
                let d = v.as_string();
                if !d.trim().is_empty() {
                    p.domain = Some(d);
                }
            }
            if let Some(v) = sc.get_ci("path") {
                let pp = v.as_string();
                if !pp.trim().is_empty() {
                    p.path = pp;
                }
            }
        }
        p
    }

    /// Render the `Set-Cookie` header *value* (everything after `Set-Cookie:`)
    /// for the session id. `conn_is_secure` is the runtime's view of whether
    /// this request arrived over a secure transport; it is only consulted when
    /// the app left `secure` unset.
    pub fn render(&self, name: &str, value: &str, conn_is_secure: bool) -> String {
        let mut out = format!("{}={}; Path={}", name, value, self.path);
        if let Some(ref d) = self.domain {
            out.push_str("; Domain=");
            out.push_str(d);
        }
        if self.httponly {
            out.push_str("; HttpOnly");
        }
        match self.samesite {
            SameSite::Lax => out.push_str("; SameSite=Lax"),
            SameSite::Strict => out.push_str("; SameSite=Strict"),
            SameSite::None => out.push_str("; SameSite=None"),
            SameSite::Unset => {}
        }
        // Secure resolves to the explicit app setting, else the connection's own
        // security. Browsers reject `SameSite=None` without `Secure`, so force
        // it on in that case to avoid silently emitting a cookie the browser
        // will drop.
        let secure = self.secure.unwrap_or(conn_is_secure) || self.samesite == SameSite::None;
        if secure {
            out.push_str("; Secure");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamic::{CfmlValue, ValueMap};

    fn cfg_with(inner: Vec<(&str, CfmlValue)>) -> ValueMap {
        let mut sc = ValueMap::default();
        for (k, v) in inner {
            sc.insert(k.to_string(), v);
        }
        let mut config = ValueMap::default();
        config.insert("sessioncookie".to_string(), CfmlValue::strukt(sc));
        config
    }

    #[test]
    fn default_policy_auto_secure() {
        let p = SessionCookiePolicy::default();
        // Plain HTTP → no Secure.
        assert_eq!(
            p.render("CFID", "abc", false),
            "CFID=abc; Path=/; HttpOnly; SameSite=Lax"
        );
        // HTTPS → Secure appears, no config needed.
        assert_eq!(
            p.render("CFID", "abc", true),
            "CFID=abc; Path=/; HttpOnly; SameSite=Lax; Secure"
        );
    }

    #[test]
    fn explicit_secure_true_overrides_plain_http() {
        let p = SessionCookiePolicy::from_app_config(&cfg_with(vec![(
            "secure",
            CfmlValue::Bool(true),
        )]));
        // moopa's case: secure:true honoured even though the connection looks
        // insecure to the CLI.
        assert!(p.render("CFID", "x", false).ends_with("; Secure"));
    }

    #[test]
    fn explicit_secure_false_overrides_https() {
        let p = SessionCookiePolicy::from_app_config(&cfg_with(vec![(
            "secure",
            CfmlValue::Bool(false),
        )]));
        // Divergence is confined to the unspecified case: an explicit false is
        // honoured on both runtimes even over HTTPS.
        assert!(!p.render("CFID", "x", true).contains("Secure"));
    }

    #[test]
    fn honours_httponly_samesite_domain_path() {
        let p = SessionCookiePolicy::from_app_config(&cfg_with(vec![
            ("httponly", CfmlValue::Bool(false)),
            ("samesite", CfmlValue::string("Strict")),
            ("domain", CfmlValue::string(".example.com")),
            ("path", CfmlValue::string("/app")),
        ]));
        let rendered = p.render("CFID", "x", false);
        assert_eq!(
            rendered,
            "CFID=x; Path=/app; Domain=.example.com; SameSite=Strict"
        );
    }

    #[test]
    fn samesite_none_forces_secure() {
        let p = SessionCookiePolicy::from_app_config(&cfg_with(vec![(
            "samesite",
            CfmlValue::string("None"),
        )]));
        // SameSite=None is dropped by browsers without Secure → force it.
        assert!(p.render("CFID", "x", false).ends_with("; SameSite=None; Secure"));
    }

    #[test]
    fn empty_samesite_omits_attribute() {
        let p = SessionCookiePolicy::from_app_config(&cfg_with(vec![(
            "samesite",
            CfmlValue::string(""),
        )]));
        assert_eq!(p.render("CFID", "x", false), "CFID=x; Path=/; HttpOnly");
    }

    #[test]
    fn absent_sessioncookie_is_default() {
        let config: ValueMap = ValueMap::default();
        let p = SessionCookiePolicy::from_app_config(&config);
        assert_eq!(p.secure, None);
        assert_eq!(p.samesite, SameSite::Lax);
        assert!(p.httponly);
    }
}
