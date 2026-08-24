//! Virtual machine types and context

use crate::dynamic::{CfmlValue, ValueMap};

pub type CfmlResult = Result<CfmlValue, CfmlError>;

/// `error_type` marking [`CfmlError::shim_unhandled`]. Deliberately not a
/// plausible CFML/Java exception name so it can never be caught by user code —
/// it is consumed by the VM's java-shim dispatch and never escapes.
const SHIM_UNHANDLED_TYPE: &str = "__rustcfml_shim_unhandled__";

#[derive(Debug, Clone)]
pub struct CfmlError {
    pub message: String,
    pub error_type: CfmlErrorType,
    pub stack_trace: Vec<StackFrame>,
    /// Extra members to merge onto the `cfcatch` struct the VM synthesises for
    /// this error, beyond the standard `message`/`type`/`detail`/`stackTrace`
    /// set. This is how structured driver detail reaches CFML — a database
    /// failure carries `SQLState`/`NativeErrorCode`/`Sql`/`DataSource` here so
    /// `catch( database e )` can branch on `e.sqlState` instead of
    /// substring-sniffing `e.message` (GitHub #295).
    ///
    /// Boxed and `None` by default: nearly every error carries no extras, and
    /// `CfmlError` is moved through every `?` on the hot path, so the field
    /// must cost one null pointer rather than an inline map.
    pub extras: Option<Box<ValueMap>>,
}

#[derive(Debug, Clone)]
pub enum CfmlErrorType {
    Runtime,
    Compile,
    Expression,
    Template,
    Application,
    Custom(String),
}

#[derive(Debug, Clone)]
pub struct StackFrame {
    pub function: String,
    pub template: String,
    pub line: usize,
}

impl CfmlError {
    pub fn new(message: String, error_type: CfmlErrorType) -> Self {
        Self {
            message,
            error_type,
            stack_trace: Vec::new(),
            extras: None,
        }
    }

    /// Attach (or merge into) the structured `cfcatch` extras — see
    /// [`CfmlError::extras`]. Later inserts win, so a driver-level call can set
    /// `SQLState` and a later call-site-level call can add `Sql`/`DataSource`
    /// without either clobbering the other's keys.
    pub fn with_extras<I>(mut self, entries: I) -> Self
    where
        I: IntoIterator<Item = (String, CfmlValue)>,
    {
        let map = self.extras.get_or_insert_with(Default::default);
        for (k, v) in entries {
            map.insert(k, v);
        }
        self
    }

    pub fn runtime(message: String) -> Self {
        Self::new(message, CfmlErrorType::Runtime)
    }

    /// A catchable `database`-typed exception, matching how Lucee/ACF surface
    /// SQL execution and connection failures. CFML code routinely does
    /// `catch( database e )` (e.g. Preside's cascade-delete guard depends on a
    /// FK-constraint violation arriving as a `database` exception), so DB errors
    /// must NOT be generic `runtime` errors.
    pub fn database(message: String) -> Self {
        Self::new(message, CfmlErrorType::Custom("database".to_string()))
    }

    /// A catchable `lock`-typed exception, matching how Lucee surfaces `<cflock>`
    /// failures (a timeout, or an invalid attribute combination). CFML code does
    /// `catch( lock e )` to distinguish contention from a genuine error inside the
    /// locked body, so these must not arrive as generic `runtime` errors.
    pub fn lock(message: String) -> Self {
        Self::new(message, CfmlErrorType::Custom("lock".to_string()))
    }

    /// A catchable `application`-typed exception, matching how Lucee surfaces a
    /// failure to launch an external process from `<cfexecute>`. CFML code does
    /// `catch( application e )` around a shell-out, so a missing or
    /// non-executable binary must not arrive as a generic `runtime` error.
    pub fn application(message: String) -> Self {
        Self::new(message, CfmlErrorType::Custom("application".to_string()))
    }

    /// A catchable `template`-typed exception, matching Lucee's
    /// `lucee.runtime.exp.TemplateException` — raised when a tag is used in a
    /// context that cannot support it (e.g. `<cfexit method="loop">` outside a
    /// custom tag's end phase).
    pub fn template(message: String) -> Self {
        Self::new(message, CfmlErrorType::Template)
    }

    /// A catchable `expression`-typed exception, matching how Lucee/ACF surface
    /// expression-evaluation errors (e.g. an invalid datepart passed to
    /// `dateAdd`/`dateDiff`). CFML code does `catch( expression e )` and many
    /// guards `try{}catch(any e){}` around these, so they must arrive typed.
    pub fn expression(message: String) -> Self {
        Self::new(message, CfmlErrorType::Expression)
    }

    /// A missing-file exception whose `type` matches Java's
    /// `java.io.FileNotFoundException`, the way Lucee/ACF surface a missing file
    /// from `fileRead*`. CFML code branches on it — e.g. Preside's
    /// `FileSystemStorageProvider.getObject` does
    /// `if ( e.type contains "FileNotFoundException" ) { throw objectNotFound }`.
    pub fn file_not_found(message: String) -> Self {
        Self::new(
            message,
            CfmlErrorType::Custom("java.io.FileNotFoundException".to_string()),
        )
    }

    /// An I/O exception whose `type` matches Java's `java.io.IOException`, the
    /// way Lucee/ACF surface an unreadable/undecodable image from `imageNew`/
    /// `imageRead` (their `ImageIO.read` throws `IOException`). CFML code
    /// branches on it — e.g. Preside's `NativeImageService.resize` does
    /// `try{ ImageNew(binary) }catch("java.io.IOException"){ throw notAnImage }`.
    pub fn io_exception(message: String) -> Self {
        Self::new(
            message,
            CfmlErrorType::Custom("java.io.IOException".to_string()),
        )
    }

    /// Internal marker: a java shim was asked for a method it does not
    /// implement. NOT a user-visible error — the VM's shim dispatch catches it
    /// and falls through to generic dispatch.
    ///
    /// This exists because the old convention was "a shim returning `Ok(Null)`
    /// means it didn't handle the method". That conflates *unhandled* with
    /// *handled, returned null*, and every method that legitimately returns
    /// null had to be enumerated in a hand-maintained allowlist
    /// (`map_getter_owns_null`) that grew with each bug: GH #218, #239, #249,
    /// #276. With unhandled carried out-of-band, `Ok(Null)` from a shim is
    /// always authoritative and the allowlist disappears.
    pub fn shim_unhandled(method: &str) -> Self {
        Self::new(
            format!("__shim_unhandled__:{}", method),
            CfmlErrorType::Custom(SHIM_UNHANDLED_TYPE.to_string()),
        )
    }

    /// Is this the internal "shim did not handle this method" marker?
    pub fn is_shim_unhandled(&self) -> bool {
        matches!(&self.error_type, CfmlErrorType::Custom(t) if t == SHIM_UNHANDLED_TYPE)
    }

    /// An unknown-hash-algorithm exception whose `type` matches Java's
    /// `java.security.NoSuchAlgorithmException` — what Lucee reports from both
    /// `hash(input, "<unknown>")` and
    /// `MessageDigest.getInstance("<unknown>")` (verified on Lucee 7.0.4:
    /// `bogus-alg MessageDigest not available`). Both used to fall back to MD5
    /// silently, which downgrades a caller who asked for SHA-512 to a broken
    /// digest without a word.
    pub fn no_such_algorithm(algorithm: &str) -> Self {
        Self::new(
            format!("{} MessageDigest not available", algorithm),
            CfmlErrorType::Custom("java.security.NoSuchAlgorithmException".to_string()),
        )
    }
}

impl CfmlErrorType {
    /// The CFML-canonical `e.type` string reported to caught CFML code — as
    /// opposed to the human-facing `Display` form used for error banners.
    /// `Expression` maps to lowercase `expression`, the value Lucee/ACF report
    /// and the value the VM's in-handler (same-frame) undefined-read paths
    /// already hardcode. Without this, an undefined read that propagated across
    /// a call frame surfaced as `Expression` (Display-cased) while the same read
    /// at page scope surfaced as `expression` (GH #282). Other categories keep
    /// their existing casing — notably `Application`, which the default `throw`
    /// type and tests depend on being capitalized.
    pub fn type_name(&self) -> String {
        match self {
            CfmlErrorType::Expression => "expression".to_string(),
            other => other.to_string(),
        }
    }
}

impl std::fmt::Display for CfmlErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CfmlErrorType::Runtime => write!(f, "Runtime"),
            CfmlErrorType::Compile => write!(f, "Compile"),
            CfmlErrorType::Expression => write!(f, "Expression"),
            CfmlErrorType::Template => write!(f, "Template"),
            CfmlErrorType::Application => write!(f, "Application"),
            CfmlErrorType::Custom(s) => write!(f, "{}", s),
        }
    }
}

impl std::fmt::Display for CfmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} Error: {}", self.error_type, self.message)?;
        if !self.stack_trace.is_empty() {
            write!(f, "\n\nStack trace (most recent call first):")?;
            for (i, frame) in self.stack_trace.iter().enumerate() {
                let template = if frame.template.is_empty() { "<inline>" } else { &frame.template };
                let func = if frame.function == "__main__" { "(main)" } else { &frame.function };
                write!(f, "\n  {}: {} ({}:{})", i + 1, func, template, frame.line)?;
            }
        }
        Ok(())
    }
}

pub struct CfmlContext {
    pub scopes: Vec<ValueMap>,
    pub this: Option<CfmlValue>,
    pub super_scope: Option<CfmlValue>,
    pub variables: ValueMap,
    pub local_vars: ValueMap,
    pub output_buffer: String,
    pub output_enabled: bool,
}

impl CfmlContext {
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            this: None,
            super_scope: None,
            variables: ValueMap::default(),
            local_vars: ValueMap::default(),
            output_buffer: String::new(),
            output_enabled: true,
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(ValueMap::default());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn get_var(&self, name: &str) -> Option<CfmlValue> {
        for scope in self.scopes.iter().rev() {
            if let Some(val) = scope.get(name) {
                return Some(val.clone());
            }
        }
        self.local_vars
            .get(name)
            .cloned()
            .or_else(|| self.variables.get(name).cloned())
    }

    pub fn set_var(&mut self, name: String, value: CfmlValue) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, value);
        } else {
            self.variables.insert(name, value);
        }
    }

    pub fn write_output(&mut self, value: &str) {
        if self.output_enabled {
            self.output_buffer.push_str(value);
        }
    }
}

impl Default for CfmlContext {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CfmlFrame {
    pub name: String,
    pub ip: usize,
    pub stack: Vec<CfmlValue>,
    pub locals: ValueMap,
}

impl CfmlFrame {
    pub fn new(name: String) -> Self {
        Self {
            name,
            ip: 0,
            stack: Vec::new(),
            locals: ValueMap::default(),
        }
    }
}
