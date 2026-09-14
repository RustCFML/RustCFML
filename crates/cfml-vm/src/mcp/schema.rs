//! CFML function signatures → JSON Schema.
//!
//! An MCP client only knows how to call a tool because of its `inputSchema`,
//! and hand-writing JSON Schema is the part of authoring an MCP server that
//! everyone gets wrong. CFML already declares everything needed — parameter
//! names, types, `required`, defaults, and `hint`/`@param.description`
//! annotations — so the schema is derived, not written. An explicit
//! `inputSchema="{…}"` annotation on the function still wins when a tool needs
//! something the CFML type system cannot express (enums, nested objects).

use cfml_common::dynamic::{CfmlParam, CfmlValue};
use serde_json::{json, Map, Value};

use super::content;

/// Map a CFML parameter type to its JSON Schema type. CFML's type vocabulary
/// is wider than JSON's; anything that is really "a string with a format"
/// (date, uuid, email, …) is reported as `string` so a client can always
/// produce a valid value.
fn json_type(cfml_type: Option<&str>) -> Option<(&'static str, Option<&'static str>)> {
    let t = cfml_type?.trim().to_lowercase();
    Some(match t.as_str() {
        "string" | "char" => ("string", None),
        "numeric" | "number" | "double" | "float" | "int" | "integer" | "long" => {
            ("number", None)
        }
        "boolean" | "bool" => ("boolean", None),
        "array" | "query" => ("array", None),
        "struct" => ("object", None),
        "date" | "datetime" => ("string", Some("date-time")),
        "uuid" | "guid" => ("string", Some("uuid")),
        "email" => ("string", Some("email")),
        "url" => ("string", Some("uri")),
        // `any`, `variablename`, a component path, or an unrecognised type:
        // emit no `type` at all rather than guessing. An absent type is valid
        // JSON Schema meaning "anything", which is honest.
        _ => return None,
    })
}

/// Read a parameter's human description: the `@name.description` doc-comment
/// annotation, or the `hint` attribute, whichever is present.
fn param_description(param: &CfmlParam) -> Option<String> {
    param
        .annotations
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("description") || k.eq_ignore_ascii_case("hint"))
        .map(|(_, v)| v.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Build the `inputSchema` object for a tool from its declared parameters.
///
/// A parameter is required when CFML says it is **and** it has no default —
/// a defaulted parameter is by definition optional to the caller, even if the
/// author also wrote `required`.
pub fn input_schema(params: &[CfmlParam]) -> Value {
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();

    for p in params {
        let mut prop = Map::new();
        if let Some((ty, format)) = json_type(p.param_type.as_deref()) {
            prop.insert("type".into(), json!(ty));
            if let Some(f) = format {
                prop.insert("format".into(), json!(f));
            }
        }
        if let Some(d) = param_description(p) {
            prop.insert("description".into(), json!(d));
        }
        if let Some(default) = &p.default {
            // Null defaults carry no information and clutter the schema.
            if !matches!(default, CfmlValue::Null) {
                prop.insert("default".into(), content::to_json(default));
            }
        }
        properties.insert(p.name.clone(), Value::Object(prop));

        if p.required && p.default.is_none() {
            required.push(json!(p.name));
        }
    }

    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    // The key must be present even when empty: some clients treat an absent
    // `required` differently from an empty one.
    schema.insert("required".into(), Value::Array(required));
    Value::Object(schema)
}

/// Parse an explicit `inputSchema="{…}"` / `outputSchema="{…}"` annotation.
/// Invalid JSON returns `None` so the caller falls back to the derived schema
/// rather than serving a broken one.
pub fn explicit_schema(raw: &str) -> Option<Value> {
    let parsed: Value = serde_json::from_str(raw.trim()).ok()?;
    parsed.is_object().then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, ty: Option<&str>, required: bool, default: Option<CfmlValue>) -> CfmlParam {
        CfmlParam {
            name: name.to_string(),
            param_type: ty.map(|t| t.to_string()),
            default,
            required,
            annotations: Vec::new(),
        }
    }

    #[test]
    fn derives_types_and_required_list() {
        let schema = input_schema(&[
            param("query", Some("string"), true, None),
            param("limit", Some("numeric"), false, Some(CfmlValue::Int(10))),
        ]);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["query"]["type"], "string");
        assert_eq!(schema["properties"]["limit"]["type"], "number");
        assert_eq!(schema["properties"]["limit"]["default"], 10);
        assert_eq!(schema["required"], json!(["query"]));
    }

    #[test]
    fn a_defaulted_param_is_never_required() {
        // `required string x = "a"` is contradictory; the default wins, because
        // the caller genuinely may omit it.
        let schema = input_schema(&[param("x", Some("string"), true, Some(CfmlValue::string("a")))]);
        assert_eq!(schema["required"], json!([]));
    }

    #[test]
    fn unknown_and_any_types_emit_no_type_keyword() {
        let schema = input_schema(&[param("x", Some("any"), false, None), param("y", None, false, None)]);
        assert!(schema["properties"]["x"].get("type").is_none());
        assert!(schema["properties"]["y"].get("type").is_none());
    }

    #[test]
    fn date_is_a_formatted_string() {
        let schema = input_schema(&[param("when", Some("date"), false, None)]);
        assert_eq!(schema["properties"]["when"]["type"], "string");
        assert_eq!(schema["properties"]["when"]["format"], "date-time");
    }

    #[test]
    fn description_comes_from_annotation_or_hint() {
        let mut p = param("q", Some("string"), true, None);
        p.annotations.push(("description".into(), "Search terms".into()));
        let schema = input_schema(&[p]);
        assert_eq!(schema["properties"]["q"]["description"], "Search terms");

        let mut p = param("q", Some("string"), true, None);
        p.annotations.push(("hint".into(), "From hint".into()));
        assert_eq!(input_schema(&[p])["properties"]["q"]["description"], "From hint");
    }

    #[test]
    fn no_params_still_yields_a_valid_object_schema() {
        let schema = input_schema(&[]);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!([]));
        assert!(schema["properties"].is_object());
    }

    #[test]
    fn explicit_schema_must_be_a_json_object() {
        assert!(explicit_schema(r#"{"type":"object"}"#).is_some());
        assert!(explicit_schema("[1,2]").is_none());
        assert!(explicit_schema("not json").is_none());
    }
}
