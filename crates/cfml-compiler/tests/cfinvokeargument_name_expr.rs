//! Regression for GH #545: `<cfinvokeargument name="#expr#">` must evaluate
//! `name` like any other interpolated attribute. Previously `strip_hashes`
//! turned the expression text into a quoted literal key — so `#n#` became the
//! key `"n"`, `x#n#` became `"xn"`, and nested double quotes inside `#...#`
//! produced unparseable script (`"listFirst( key, "=" )"`).
//!
//! Keys are emitted as `("" & <expr>)` so a bare identifier from `#n#` is a
//! computed key, not a literal struct-key name.

use cfml_compiler::tag_parser::tags_to_script;

#[test]
fn static_name_still_string_key() {
    let s = tags_to_script(
        r##"<cfinvoke method="f" returnvariable="r"><cfinvokeargument name="a" value="1"></cfinvoke>"##,
    );
    assert!(
        s.contains(r#"("" & "a"): "1""#),
        "static name must stay a string-key expression: {s}"
    );
}

#[test]
fn dollar_name_still_string_key() {
    let s = tags_to_script(
        r##"<cfinvoke method="f" returnvariable="r"><cfinvokeargument name="$" value="#scope#"></cfinvoke>"##,
    );
    assert!(
        s.contains(r#"("" & "$"): scope"#),
        "literal $ name must remain a string key with value as expression: {s}"
    );
}

#[test]
fn name_hash_variable_evaluates() {
    let s = tags_to_script(
        r##"<cfinvoke method="f" returnvariable="r"><cfinvokeargument name="#n#" value="1"></cfinvoke>"##,
    );
    assert!(
        s.contains(r#"("" & n): "1""#),
        "name=\"#n#\" must evaluate n, not use literal key \"n\": {s}"
    );
    assert!(
        !s.contains("\"n\":"),
        "must not emit a quoted literal key \"n\": {s}"
    );
}

#[test]
fn name_prefix_plus_interpolation_evaluates() {
    let s = tags_to_script(
        r##"<cfinvoke method="f" returnvariable="r"><cfinvokeargument name="x#n#" value="1"></cfinvoke>"##,
    );
    assert!(
        s.contains(r#"("" & "x" & (n)): "1""#),
        "name=\"x#n#\" must concatenate literal x with evaluated n: {s}"
    );
    assert!(
        !s.contains("\"xn\""),
        "must not collapse x#n# into literal key xn: {s}"
    );
}

#[test]
fn name_listfirst_single_quoted_delimiter_evaluates() {
    let s = tags_to_script(
        r##"<cfinvoke method="f" returnvariable="r"><cfinvokeargument name="#listFirst( key, '=' )#" value="1"></cfinvoke>"##,
    );
    assert!(
        s.contains(r#"("" & listFirst( key, '=' )): "1""#),
        "name expression must evaluate listFirst: {s}"
    );
    assert!(
        !s.contains("\"listFirst"),
        "expression must not be a quoted literal key: {s}"
    );
}

#[test]
fn name_listfirst_double_quoted_delimiter_evaluates() {
    let s = tags_to_script(
        r##"<cfinvoke method="f" returnvariable="r"><cfinvokeargument name="#listFirst( key, "=" )#" value="1"></cfinvoke>"##,
    );
    assert!(
        s.contains(r#"("" & listFirst( key, "=" )): "1""#),
        "nested double quotes inside #...# must survive as expression text: {s}"
    );
}

#[test]
fn value_with_nested_double_quotes_still_evaluates() {
    let s = tags_to_script(
        r##"<cfinvoke method="f" returnvariable="r"><cfinvokeargument name="b" value="#listLast( key, "=" )#"></cfinvoke>"##,
    );
    assert!(
        s.contains(r#"("" & "b"): listLast( key, "=" )"#),
        "value interpolation with nested quotes must keep working: {s}"
    );
}
