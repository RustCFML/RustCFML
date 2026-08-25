//! `javax.mail.Session` / `Transport` / `java.util.Properties`-driven SMTP probe.
//!
//! CFML can *send* mail (`<cfmail>`) but has never been able to *test* a mail
//! server, so any admin screen with a "verify these SMTP settings" button drops to
//! JavaMail. Preside's `EmailService.validateConnectionSettings()` is the shape:
//!
//! ```cfml
//! props = CreateObject( "java", "java.util.Properties" ).init();
//! props.put( "mail.smtp.starttls.enable", "true" );
//! props.put( "mail.smtp.auth", "true" );
//!
//! mailSession = CreateObject( "java", "javax.mail.Session" ).getInstance( props, NullValue() );
//! transport   = mailSession.getTransport( "smtp" );
//! try {
//!     transport.connect( host, port, username, password );
//! } catch ( "javax.mail.AuthenticationFailedException" e ) { … }
//!   catch ( any e ) { … }
//!   finally { transport.close(); }
//! ```
//!
//! The probe itself is the `smtpConnectionTest()` builtin. This module carries the
//! session properties across to it and, crucially, **throws the exception types
//! the caller catches** — a bad password must arrive as
//! `javax.mail.AuthenticationFailedException`, not as a generic error, or the
//! `catch` above silently reports the wrong cause to the user.
//!
//! Properties honoured, matching JavaMail's names:
//! `mail.smtp.starttls.enable`, `mail.smtp.ssl.enable`, `mail.smtp.host`,
//! `mail.smtp.port`, `mail.smtp.user`, `mail.smtp.connectiontimeout` /
//! `mail.smtp.timeout` (milliseconds, as JavaMail specifies).
//!
//! Sending is **not** shimmed: `Transport.send( message )` needs the whole
//! `MimeMessage` object graph, and CFML already has `<cfmail>` for that. It throws.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const SESSION_CLASS: &str = "javax.mail.session";
pub const TRANSPORT_CLASS: &str = "javax.mail.transport";

pub fn is_mail_class(class_lower: &str) -> bool {
    matches!(class_lower, SESSION_CLASS | TRANSPORT_CLASS)
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

pub fn construct(class_lower: &str) -> CfmlResult {
    Ok(CfmlValue::strukt(shim(class_lower)))
}

fn field(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}

fn field_str(object: &CfmlValue, key: &str) -> String {
    field(object, key).map(|v| v.as_string()).unwrap_or_default()
}

/// Read one `mail.*` property off the carried Properties map (case-insensitively,
/// since a CFML struct upper-cases its keys on some engines).
fn prop(object: &CfmlValue, name: &str) -> Option<String> {
    let props = field(object, "__props")?;
    let CfmlValue::Struct(s) = props else {
        return None;
    };
    s.get_ci(name).map(|v| v.as_string()).filter(|v| !v.is_empty())
}

fn prop_bool(object: &CfmlValue, name: &str, default: bool) -> bool {
    match prop(object, name) {
        Some(v) => v.eq_ignore_ascii_case("true") || v == "1",
        None => default,
    }
}

fn auth_failed(message: &str) -> CfmlError {
    CfmlError::new(
        format!("javax.mail.AuthenticationFailedException: {}", message),
        CfmlErrorType::Custom("javax.mail.AuthenticationFailedException".to_string()),
    )
}

fn messaging_exception(message: &str) -> CfmlError {
    CfmlError::new(
        format!("javax.mail.MessagingException: {}", message),
        CfmlErrorType::Custom("javax.mail.MessagingException".to_string()),
    )
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "{}.{}() is not supported by RustCFML's JavaMail shim, which covers the \
             connection probe (Session.getInstance → getTransport → connect/close). \
             Use <cfmail> to send.",
            class, method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

pub fn handle_session(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        // getInstance( props [, authenticator] ) and getDefaultInstance( … ).
        // The authenticator is ignored: it exists to prompt for credentials the
        // probe is given directly.
        "getinstance" | "getdefaultinstance" | "init" => {
            let mut m = shim(SESSION_CLASS);
            if let Some(p) = args.first() {
                m.insert("__props".to_string(), p.clone());
            }
            Ok(CfmlValue::strukt(m))
        }
        "gettransport" => {
            let protocol = args
                .first()
                .map(|v| v.as_string())
                .unwrap_or_else(|| "smtp".to_string());
            if !protocol.eq_ignore_ascii_case("smtp") && !protocol.eq_ignore_ascii_case("smtps") {
                return Err(messaging_exception(&format!(
                    "no provider for protocol '{}' — RustCFML's shim speaks SMTP only",
                    protocol
                )));
            }
            let mut m = shim(TRANSPORT_CLASS);
            // Carry the session's properties down: connect() reads TLS and
            // timeout settings from them.
            if let Some(p) = field(object, "__props") {
                m.insert("__props".to_string(), p);
            }
            m.insert(
                "__implicit_tls".to_string(),
                CfmlValue::Bool(protocol.eq_ignore_ascii_case("smtps")),
            );
            Ok(CfmlValue::strukt(m))
        }
        "getproperty" => Ok(match args.first().map(|v| v.as_string()) {
            Some(name) => match prop(object, &name) {
                Some(v) => CfmlValue::string(v),
                None => CfmlValue::Null,
            },
            None => CfmlValue::Null,
        }),
        "setdebug" => Ok(CfmlValue::Null),
        other => Err(unsupported("javax.mail.Session", other)),
    }
}

/// `probe` is the `smtpConnectionTest()` builtin.
pub fn handle_transport(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    probe: impl FnOnce(Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    match method {
        // connect() | connect(host, user, pass) | connect(host, port, user, pass)
        "connect" => {
            let (host, port, user, pass) = match args.len() {
                0 => (
                    prop(object, "mail.smtp.host").unwrap_or_default(),
                    prop(object, "mail.smtp.port").unwrap_or_default(),
                    prop(object, "mail.smtp.user").unwrap_or_default(),
                    String::new(),
                ),
                3 => (
                    args[0].as_string(),
                    prop(object, "mail.smtp.port").unwrap_or_default(),
                    args[1].as_string(),
                    args[2].as_string(),
                ),
                _ => (
                    args[0].as_string(),
                    args.get(1).map(|v| v.as_string()).unwrap_or_default(),
                    args.get(2).map(|v| v.as_string()).unwrap_or_default(),
                    args.get(3).map(|v| v.as_string()).unwrap_or_default(),
                ),
            };
            if host.trim().is_empty() {
                return Err(messaging_exception(
                    "connect() needs a host, either as an argument or as mail.smtp.host",
                ));
            }
            let port = if port.trim().is_empty() {
                // JavaMail's defaults: 465 for smtps, 25 otherwise.
                if matches!(field(object, "__implicit_tls"), Some(CfmlValue::Bool(true))) {
                    "465".to_string()
                } else {
                    "25".to_string()
                }
            } else {
                port
            };

            let implicit_tls = matches!(field(object, "__implicit_tls"), Some(CfmlValue::Bool(true)))
                || prop_bool(object, "mail.smtp.ssl.enable", false);
            let starttls = prop_bool(object, "mail.smtp.starttls.enable", false);
            // JavaMail timeouts are milliseconds; the BIF takes seconds.
            let timeout_secs = prop(object, "mail.smtp.connectiontimeout")
                .or_else(|| prop(object, "mail.smtp.timeout"))
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(|ms| (ms / 1000).max(1))
                .unwrap_or(10);

            let result = probe(vec![
                CfmlValue::string(host.clone()),
                CfmlValue::string(port.clone()),
                CfmlValue::string(user),
                CfmlValue::string(pass),
                CfmlValue::Bool(starttls),
                CfmlValue::Bool(implicit_tls),
                CfmlValue::Int(timeout_secs as i64),
            ])?;

            let ok = matches!(field(&result, "success"), Some(CfmlValue::Bool(true)));
            if ok {
                if let CfmlValue::Struct(s) = object {
                    s.insert("__connected".to_string(), CfmlValue::Bool(true));
                }
                return Ok(CfmlValue::Null);
            }
            let message = field_str(&result, "message");
            // The distinction the caller's `catch` branches on.
            if matches!(field(&result, "authFailed"), Some(CfmlValue::Bool(true))) {
                Err(auth_failed(&message))
            } else {
                Err(messaging_exception(&message))
            }
        }
        "isconnected" => Ok(CfmlValue::Bool(matches!(
            field(object, "__connected"),
            Some(CfmlValue::Bool(true))
        ))),
        // Called in a `finally`, so it must succeed even when connect() threw —
        // the probe holds no socket open past its own call.
        "close" => {
            if let CfmlValue::Struct(s) = object {
                s.insert("__connected".to_string(), CfmlValue::Bool(false));
            }
            Ok(CfmlValue::Null)
        }
        "sendmessage" | "send" => Err(unsupported("javax.mail.Transport", method)),
        other => Err(unsupported("javax.mail.Transport", other)),
    }
}
