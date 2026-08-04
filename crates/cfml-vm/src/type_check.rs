//! Declared parameter / return-type enforcement (docs/known-issues.md §29).
//!
//! `param_type`/`return_type` used to be carried all the way into
//! `BytecodeFunction` and then read by nothing but `getMetadata()`, so
//! `function f( required numeric n )` happily accepted `"notanumber"` and
//! `function g() returntype="numeric" { return "abc"; }` happily returned it.
//! This module is the missing consumer.
//!
//! Every rule below was probed against **Lucee 7.0.4**; the probes live in
//! `tests/functions/fn_type_enforcement_crossengine_probe*.cfm` and the
//! resulting expectations in `tests/functions/test_fn_type_enforcement.cfm`,
//! which passes on both engines. Two things about Lucee's contract are worth
//! stating up front because they are not what you would guess:
//!
//! 1. **It validates, it does not coerce.** A `numeric`-typed parameter given
//!    the string `"123"` receives `"123"` — still a string — not `123`. So
//!    this module only ever answers yes/no; it never rewrites the value.
//! 2. **A type name Lucee has no cast target for is treated as a component
//!    path**, which means it rejects *every* value. `integer`, `int`, `float`,
//!    `double`, `decimal`, `long`, `short`, `byte`, `char`, `email`,
//!    `creditcard`, `url`, `base64`, `usdate`, `eurodate`, `hex`, `path`,
//!    `node`, `closure`, `lambda` and `udf` all fall in this bucket:
//!    `function f( integer i )` throws on `f( 5 )`, and `f( email e )` throws
//!    on `f( "a@b.com" )`. Lucee contradicts its own `isValid( "integer", 5 )`
//!    here, but it is the reference and mirroring it is deliberate — see the
//!    §29 note in docs/known-issues.md. (Nothing is lost by mirroring: on
//!    Lucee such a call is unconditionally fatal, so no Lucee-tested app can
//!    contain a reachable one.)
//!
//! The messages are Lucee's, verbatim, including the ordinal quirk
//! (`first`, `second`, then `3th`, `4th`, …) and the odd space before the
//! comma in the wrapped return-value form.

use cfml_common::dynamic::CfmlValue;

/// A declared type resolved to the cast target Lucee would use.
enum Target<'a> {
    /// `any` / undeclared — never checked.
    Any,
    Str,
    Numeric,
    Boolean,
    /// `date` / `datetime` / `time` all collapse to one target.
    DateTime,
    TimeSpan,
    Array,
    Struct,
    Query,
    Binary,
    Xml,
    Function,
    Uuid,
    Guid,
    VariableName,
    /// `component` / `object` — any component instance.
    AnyComponent,
    Void,
    /// `T[]` — an array whose every element satisfies `T`.
    TypedArray(&'a str),
    /// Anything else: resolved as a component/interface name. Rejects every
    /// non-instance value, which is what makes `integer`/`email`/… always throw.
    ComponentPath,
}

fn resolve<'a>(declared: &'a str) -> Target<'a> {
    let t = declared.trim();
    if let Some(inner) = t.strip_suffix("[]") {
        return Target::TypedArray(inner);
    }
    match t.to_ascii_lowercase().as_str() {
        "" | "any" => Target::Any,
        "string" => Target::Str,
        "numeric" | "number" => Target::Numeric,
        "boolean" | "bool" => Target::Boolean,
        "date" | "datetime" | "time" => Target::DateTime,
        "timespan" => Target::TimeSpan,
        "array" => Target::Array,
        "struct" => Target::Struct,
        "query" => Target::Query,
        "binary" => Target::Binary,
        "xml" => Target::Xml,
        "function" => Target::Function,
        "uuid" => Target::Uuid,
        "guid" => Target::Guid,
        "variablename" => Target::VariableName,
        "component" | "object" => Target::AnyComponent,
        "void" => Target::Void,
        _ => Target::ComponentPath,
    }
}

/// True when a declared type needs no runtime check at all — `any`, or
/// undeclared. Lets the hot call path skip everything for the common case.
pub fn is_unchecked(declared: &str) -> bool {
    matches!(resolve(declared), Target::Any)
}

/// The canonical name Lucee uses for a type in a *return-position* message:
/// a known cast target is named in lowercase (and the whole `date`/`time`/
/// `datetime` family is named `datetime`); an unknown name is echoed as
/// declared. Argument-position messages always echo as declared.
pub fn return_type_label(declared: &str) -> String {
    match resolve(declared) {
        Target::Any => "any".to_string(),
        Target::Str => "string".to_string(),
        Target::Numeric => "numeric".to_string(),
        Target::Boolean => "boolean".to_string(),
        Target::DateTime => "datetime".to_string(),
        Target::TimeSpan => "timespan".to_string(),
        Target::Array => "array".to_string(),
        Target::Struct => "struct".to_string(),
        Target::Query => "query".to_string(),
        Target::Binary => "binary".to_string(),
        Target::Xml => "xml".to_string(),
        Target::Function => "function".to_string(),
        Target::Uuid => "uuid".to_string(),
        Target::Guid => "guid".to_string(),
        Target::VariableName => "variablename".to_string(),
        Target::AnyComponent => declared.trim().to_ascii_lowercase(),
        Target::Void => "void".to_string(),
        // `string[]`, `pkg.Widget`, `integer`, … — as written.
        Target::TypedArray(_) | Target::ComponentPath => declared.trim().to_string(),
    }
}

/// Lucee's description of the offending value in a `Cannot cast …` message:
/// a string is quoted inline, everything else is named by its type.
///
/// `component_name` is consulted for component instances so the message can
/// read `Object type [Component pkg.Widget]`; it is a closure because naming
/// an instance means a metadata read the caller is better placed to do.
pub fn value_label(value: &CfmlValue, component_name: &dyn Fn(&CfmlValue) -> Option<String>) -> String {
    match value {
        CfmlValue::String(s) => format!("String [{}]", s),
        CfmlValue::Int(_) | CfmlValue::Double(_) | CfmlValue::TimeSpan(_) => {
            "Object type [Number]".to_string()
        }
        CfmlValue::Bool(_) => "Object type [Boolean]".to_string(),
        CfmlValue::Array(_) => "Object type [Array]".to_string(),
        // Named by the cell it proxies, not as an Array — otherwise a mismatch
        // on `q.col` reported a type the value does not actually have.
        CfmlValue::QueryColumn(..) => value_label(value.query_column_scalar(), component_name),
        CfmlValue::Query(_) => "Object type [Query]".to_string(),
        CfmlValue::Binary(_) => "Object type [Binary]".to_string(),
        CfmlValue::Function(f) => format!("Object type [user defined function ({})]", f.name),
        CfmlValue::Closure(_) => "Object type [user defined function (closure)]".to_string(),
        CfmlValue::Null => "Object type [null]".to_string(),
        v if is_xml_value(v) => "Object type [XML]".to_string(),
        other => match component_name(other) {
            Some(name) => format!("Object type [Component {}]", name),
            None => "Object type [Struct]".to_string(),
        },
    }
}

/// An XML document / element / node — a struct carrying the markers
/// `xmlParse()` and friends attach (see `isXmlDoc`/`isXmlNode` in cfml-stdlib).
/// Needed both to satisfy an `xml`-typed declaration and to name the value
/// `Object type [XML]` rather than `[Struct]` in a mismatch message.
fn is_xml_value(value: &CfmlValue) -> bool {
    match value {
        CfmlValue::Struct(s) => {
            s.contains_key("__xmlDoc") || s.contains_key("xmlRoot") || s.contains_key("xmlName")
        }
        _ => false,
    }
}

/// True when the value is a string Lucee would `Caster.toDoubleValue()`.
///
/// Deliberately its own scanner rather than `isNumeric()`: the accepted set is
/// exactly `[+-]?(digits[.digits] | .digits)[e[+-]digits]`, so `" 42 "` (outer
/// whitespace is trimmed), `"+5"`, `"-5"`, `".5"` and `"1e3"` pass while `""`,
/// `"  "`, `"1,000"`, `"0x10"`, `"5px"`, `"inf"` and `"NaN"` do not — the last
/// two being why `str::parse::<f64>()` alone will not do.
fn numeric_string(s: &str) -> bool {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    let int_digits = {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        i - start
    };
    let mut frac_digits = 0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - start;
    }
    if int_digits == 0 && frac_digits == 0 {
        return false;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == bytes.len()
}

/// Lucee's `Caster.toBooleanValue()` on a string: the four words, or any
/// number (`"1.5"` is a true-ish boolean to Lucee, `"abc"` and `""` are not).
fn boolean_string(s: &str) -> bool {
    let t = s.trim();
    t.eq_ignore_ascii_case("yes")
        || t.eq_ignore_ascii_case("no")
        || t.eq_ignore_ascii_case("true")
        || t.eq_ignore_ascii_case("false")
        || numeric_string(t)
}

/// Callbacks for the checks this module cannot answer alone: component
/// identity (a metadata walk the VM owns) and the string-format predicates
/// (the `isValid` family, which lives in the builtin table).
pub struct Env<'a> {
    /// Does `value` satisfy the component/interface named `type_name`?
    pub satisfies_component: &'a dyn Fn(&CfmlValue, &str) -> bool,
    /// Is `value` a component instance of any kind?
    pub is_component: &'a dyn Fn(&CfmlValue) -> bool,
    /// The registered `isValid` builtin, used for the format-checked types
    /// (`date`, `xml`, `uuid`, `guid`, `variablename`). `None` in an embedding
    /// whose builtin table has no `isValid`, where those types answer false.
    pub is_valid: Option<fn(Vec<CfmlValue>) -> cfml_common::vm::CfmlResult>,
}

impl Env<'_> {
    fn is_valid(&self, type_name: &str, value: &CfmlValue) -> bool {
        match self.is_valid {
            Some(f) => matches!(
                f(vec![CfmlValue::string(type_name.to_string()), value.clone()]),
                Ok(CfmlValue::Bool(true))
            ),
            None => false,
        }
    }
}

/// Does `value` satisfy the declared type? Never mutates or coerces.
///
/// A `Null` value is "not supplied" in CFML and is never checked — the caller
/// filters those out before asking (an omitted argument, or a function that
/// falls off its end without returning).
pub fn satisfies(value: &CfmlValue, declared: &str, env: &Env<'_>) -> bool {
    // `q.col` is a `QueryColumn` — a PROXY that stands in for its current row's
    // value, not a collection. Lucee agrees: `isArray(q.col)` is false, and
    // every scalar context here (comparison, coercion, `Len`) already treats it
    // as that one cell. The type checker was the one place that didn't, so
    // `string function f() { return q.col; }` was rejected as "Object type
    // [Array]" — which is how Preside's `SqlSchemaVersioning.getDbVersion`
    // (`return versionRecord.version_hash`) stopped its boot under §29.
    //
    // Resolved for every target EXCEPT `array`, which keeps accepting the raw
    // QueryColumn as it always did, so nothing that passed before now fails.
    let value = match resolve(declared) {
        Target::Array => value,
        _ => value.query_column_scalar(),
    };
    match resolve(declared) {
        Target::Any => true,
        // Simple values only. Binary is accepted (Lucee casts bytes to a
        // string); every container, component and function is not.
        Target::Str => matches!(
            value,
            CfmlValue::String(_)
                | CfmlValue::Int(_)
                | CfmlValue::Double(_)
                | CfmlValue::TimeSpan(_)
                | CfmlValue::Bool(_)
                | CfmlValue::Binary(_)
        ),
        Target::Numeric | Target::TimeSpan => match value {
            CfmlValue::Int(_) | CfmlValue::Double(_) | CfmlValue::TimeSpan(_) => true,
            // A boolean IS numeric to Lucee (true -> 1).
            CfmlValue::Bool(_) => true,
            CfmlValue::String(s) => numeric_string(s),
            _ => false,
        },
        Target::Boolean => match value {
            CfmlValue::Bool(_) | CfmlValue::Int(_) | CfmlValue::Double(_) => true,
            CfmlValue::String(s) => boolean_string(s),
            _ => false,
        },
        // Dates are strings in RustCFML's value model, so this is a parse
        // check plus the numeric-serial form — `0` is a valid date to Lucee,
        // and so is the STRING `"1"`, which `isValid("date", …)` rejects.
        Target::DateTime => match value {
            CfmlValue::Int(_) | CfmlValue::Double(_) => true,
            CfmlValue::String(s) => numeric_string(s) || env.is_valid("date", value),
            _ => false,
        },
        // A struct casts to an array only when it has no non-numeric keys —
        // `{}` and `{ "1" : "a" }` pass, `{ a : 1 }` does not. The value is
        // NOT converted: the callee still receives the struct.
        Target::Array => match value {
            // Binary IS a byte array to Lucee, and satisfies `array`.
            CfmlValue::Array(_) | CfmlValue::QueryColumn(..) | CfmlValue::Binary(_) => true,
            CfmlValue::Struct(s) if !(env.is_component)(value) => {
                s.keys().iter().all(|k| numeric_string(k))
            }
            _ => false,
        },
        // A component satisfies `struct` (Lucee's instances are struct-like);
        // a query does not.
        Target::Struct => matches!(value, CfmlValue::Struct(_)) || (env.is_component)(value),
        Target::Query => matches!(value, CfmlValue::Query(_)),
        // Bytes, or anything simple Lucee can turn into bytes. A date is a
        // string here, so `binary <- now()` is accepted where Lucee rejects
        // it — a consequence of the value model, not of this check.
        Target::Binary => matches!(
            value,
            CfmlValue::Binary(_)
                | CfmlValue::String(_)
                | CfmlValue::Int(_)
                | CfmlValue::Double(_)
                | CfmlValue::TimeSpan(_)
                | CfmlValue::Bool(_)
        ),
        // An already-parsed document, or a string that parses as one.
        Target::Xml => {
            is_xml_value(value)
                || (matches!(value, CfmlValue::String(_)) && env.is_valid("xml", value))
        }
        Target::Function => matches!(value, CfmlValue::Function(_) | CfmlValue::Closure(_)),
        Target::Uuid => matches!(value, CfmlValue::String(_)) && env.is_valid("uuid", value),
        Target::Guid => matches!(value, CfmlValue::String(_)) && env.is_valid("guid", value),
        Target::VariableName => {
            matches!(value, CfmlValue::String(_)) && env.is_valid("variablename", value)
        }
        Target::AnyComponent => (env.is_component)(value),
        // `returntype="void"` tolerates only "returned nothing", and the
        // caller has already filtered Null out, so any value here is wrong.
        Target::Void => false,
        Target::TypedArray(inner) => match value {
            CfmlValue::Array(a) => a.iter().all(|el| {
                // An element that is itself absent can't be type-checked.
                matches!(el, CfmlValue::Null) || satisfies(&el, inner, env)
            }),
            _ => false,
        },
        Target::ComponentPath => (env.satisfies_component)(value, declared.trim()),
    }
}

/// Lucee's ordinal words for the argument position in a type error: the first
/// two are spelled out, the rest get a bare `Nth` — including the
/// ungrammatical `3th`, which is Lucee's, not a typo.
pub fn ordinal(index_zero_based: usize) -> String {
    match index_zero_based {
        0 => "first".to_string(),
        1 => "second".to_string(),
        n => format!("{}th", n + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_strings_match_lucee() {
        for ok in ["1", "123", " 42 ", "1.5", "+5", "-5", ".5", "1e3", "1E-3", "0"] {
            assert!(numeric_string(ok), "{ok} should be numeric");
        }
        for bad in ["", "  ", "1,000", "0x10", "5px", "abc", "inf", "NaN", "1e", ".", "+"] {
            assert!(!numeric_string(bad), "{bad} should not be numeric");
        }
    }

    #[test]
    fn boolean_strings_match_lucee() {
        for ok in ["yes", "NO", "true", "False", "1", "0", "2", "-1", "1.5"] {
            assert!(boolean_string(ok), "{ok} should be boolean");
        }
        for bad in ["", "abc", "y", "n"] {
            assert!(!boolean_string(bad), "{bad} should not be boolean");
        }
    }

    #[test]
    fn ordinals_include_lucees_3th() {
        assert_eq!(ordinal(0), "first");
        assert_eq!(ordinal(1), "second");
        assert_eq!(ordinal(2), "3th");
        assert_eq!(ordinal(7), "8th");
    }

    #[test]
    fn return_labels_canonicalize_known_types_only() {
        assert_eq!(return_type_label("DATE"), "datetime");
        assert_eq!(return_type_label("time"), "datetime");
        assert_eq!(return_type_label("Numeric"), "numeric");
        assert_eq!(return_type_label("integer"), "integer");
        assert_eq!(return_type_label("string[]"), "string[]");
        assert_eq!(return_type_label("pkg.Widget"), "pkg.Widget");
    }

    #[test]
    fn unchecked_covers_any_and_undeclared() {
        assert!(is_unchecked(""));
        assert!(is_unchecked("any"));
        assert!(is_unchecked("ANY"));
        assert!(!is_unchecked("numeric"));
    }
}
