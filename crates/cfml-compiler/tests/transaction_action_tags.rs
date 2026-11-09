use cfml_compiler::{parser::Parser, tag_parser};

fn compile(source: &str) {
    let script = tag_parser::tags_to_script_checked(source).unwrap();
    Parser::new(script).parse().unwrap();
}

#[test]
fn commit_and_rollback_tags_inside_a_block_are_statements() {
    // Unclosed action tags: valid on Lucee/ACF, used to read as nested blocks.
    compile(
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
    // Self-closed.
    compile(
        r#"<cftransaction action="begin">
        <cfset x = 1>
        <cftransaction action="commit" />
    </cftransaction>"#,
    );
}

#[test]
fn a_real_nested_block_still_nests() {
    compile(
        r#"<cftransaction>
        <cftransaction action="begin"><cfset x = 1></cftransaction>
        <cfset y = 2>
    </cftransaction>"#,
    );
}

#[test]
fn an_unclosed_block_is_still_an_error() {
    assert!(tag_parser::tags_to_script_checked(
        r#"<cftransaction><cfset x = 1><cftransaction action="commit">"#
    )
    .is_err());
}
