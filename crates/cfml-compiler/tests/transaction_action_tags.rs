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
    assert!(script.contains("__cftransaction_action(act, \"\")"), "{script}");
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
fn savepoint_actions_compile_to_run_time_actions() {
    for (src, call) in [
        (
            r#"<cftransaction><cftransaction action="setsavepoint" savepoint="sp1"></cftransaction>"#,
            r#"__cftransaction_action("setsavepoint", "sp1")"#,
        ),
        (
            r#"<cftransaction><cftransaction action="rollback" savepoint="sp1" /></cftransaction>"#,
            r#"__cftransaction_action("rollback", "sp1")"#,
        ),
        (
            r##"<cftransaction><cftransaction action="rollback" savepoint="#sp#" /></cftransaction>"##,
            r#"__cftransaction_action("rollback", sp)"#,
        ),
    ] {
        let script = compile(src);
        assert!(script.contains(call), "{script}");
        assert_eq!(script.matches("__cftransaction_start(").count(), 1);
    }
}

#[test]
fn a_top_level_action_tag_with_a_body_is_a_block_that_starts_with_the_action() {
    // Lucee 7.1: `<cftransaction action="commit">body</cftransaction>` with no
    // enclosing transaction runs the body as a transaction.
    let script = compile(r#"<cftransaction action="commit"><cfset x = 1></cftransaction>"#);
    assert_eq!(script.matches("__cftransaction_start(").count(), 1, "{script}");
    let action = script.find(r#"__cftransaction_action("commit", "")"#).expect(&script);
    assert!(action > script.find("__cftransaction_start(").unwrap(), "{script}");
}

#[test]
fn an_unclosed_block_is_still_an_error() {
    assert!(tag_parser::tags_to_script_checked(
        r#"<cftransaction><cfset x = 1><cftransaction action="commit">"#
    )
    .is_err());
}
