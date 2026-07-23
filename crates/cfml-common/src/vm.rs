//! Virtual machine types and context

use crate::dynamic::{CfmlValue, ValueMap};

pub type CfmlResult = Result<CfmlValue, CfmlError>;

#[derive(Debug, Clone)]
pub struct CfmlError {
    pub message: String,
    pub error_type: CfmlErrorType,
    pub stack_trace: Vec<StackFrame>,
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
        }
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
