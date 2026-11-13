//! Which `<cftransaction>` tags have a body. Lucee's rule
//! (`transformer.cfml.attributes.impl.Transaction`): a cftransaction inside
//! another one is bodyless whenever it has an `action` attribute, whatever the
//! value; `/>` is bodyless for any tag. Every form here was probed on Lucee 7.1.

use cfml_compiler::{parser::Parser, tag_parser};

fn compile(source: &str) -> String {
    let script = tag_parser::tags_to_script_checked(source).unwrap();
    Parser::new(script.clone()).parse().unwrap();
    script
}

#[test]
fn commit_and_rollback_tags_inside_a_block_are_statements() {
    // Unclosed action tags, quoted either way and spaced around `=`.
    let script = compile(
        r#"<cftransaction isolation="serializable">
        <cftry>
            <cfset x = 1>
            <cftransaction action="commit">
            <cfcatch type="any">
                <cftransaction action = 'rollback'>
            </cfcatch>
        </cftry>
    </cftransaction>"#,
    );
    assert_eq!(script.matches("__cftransaction_start(").count(), 1);
    // Self-closed.
    compile(
        r#"<cftransaction action="begin">
        <cfset x = 1>
        <cftransaction action="commit" />
    </cftransaction>"#,
    );
}

#[test]
fn an_unquoted_action_is_still_an_action() {
    compile("<cftransaction><cfset x = 1><cftransaction action=commit></cftransaction>");
}

#[test]
fn a_nested_begin_with_an_action_attribute_has_no_body() {
    // Lucee: one </cftransaction> closes the OUTER block.
    let script = compile(
        r#"<cftransaction><cftransaction action="begin"><cfset x = 1></cftransaction>"#,
    );
    assert_eq!(script.matches("__cftransaction_start(").count(), 1);
}

#[test]
fn a_dynamic_nested_action_is_dispatched_at_run_time() {
    let script = compile(
        r##"<cfset act = "rollback"><cftransaction><cftransaction action="#act#"></cftransaction>"##,
    );
    assert_eq!(script.matches("__cftransaction_start(").count(), 1);
    assert!(script.contains("__cftxn_action"), "{script}");
}

#[test]
fn a_nested_block_without_an_action_still_nests() {
    let script = compile(
        r#"<cftransaction>
        <cftransaction><cfset x = 1></cftransaction>
        <cfset y = 2>
    </cftransaction>"#,
    );
    assert_eq!(script.matches("__cftransaction_start(").count(), 2);
}

#[test]
fn savepoints_compile_to_a_clear_not_supported_error() {
    for src in [
        r#"<cftransaction><cftransaction action="setsavepoint" savepoint="sp1"></cftransaction>"#,
        r#"<cftransaction><cftransaction action="rollback" savepoint="sp1" /></cftransaction>"#,
    ] {
        let script = compile(src);
        assert!(script.contains("savepoints"), "{script}");
        assert_eq!(script.matches("__cftransaction_start(").count(), 1);
    }
}

#[test]
fn an_unclosed_block_is_still_an_error() {
    assert!(tag_parser::tags_to_script_checked(
        r#"<cftransaction><cfset x = 1><cftransaction action="commit">"#
    )
    .is_err());
}
