use cfml_compiler::{parser::Parser, tag_parser};

fn compile(source: &str) {
    let script = if tag_parser::has_cfml_tags(source) {
        tag_parser::tags_to_script_checked(source).unwrap()
    } else {
        source.to_string()
    };
    Parser::new(script).parse().unwrap();
}

#[test]
fn script_component_remains_script() {
    let source = r#"component { function f() { ```<cfset local.x = 42><cfreturn local.x>``` } }"#;
    assert!(!tag_parser::has_cfml_tags(source));
    compile(source);
}

#[test]
fn nested_tags_queries_and_script_resume() {
    compile(
        r##"<cfscript>
        function f() {
            var q = queryNew("id", "integer", [[1]]);
            ```
            <cfquery name="local.result" dbtype="query">
                SELECT id FROM q WHERE id = <cfqueryparam value="1" cfsqltype="integer">
            </cfquery>
            <cfif local.result.recordCount><cfloop from="1" to="2" index="local.i">
                <cfset local.x = local.i>
            </cfloop></cfif>
            ```
            return local.result.recordCount;
        }
    </cfscript>"##,
    );
}

#[test]
fn delimiters_in_strings_comments_and_attributes_are_literal() {
    compile(
        r##"component {
        // ``` <cfset bad = true>
        /* ``` <cfset bad = true> */
        <!--- ``` <!--- nested ``` ---> --->
        function f() {
            var s = "``` <cfset bad = true>";
            ```<!--- ``` ---><cfset local.s = "```"><cfoutput>don't stop</cfoutput>```
            return s;
        }
    }"##,
    );
}

#[test]
fn nested_script_can_contain_another_island() {
    compile(
        r##"<cfscript>
        ```<cfscript>
            s = "</cfscript> ```";
            ```<cfset x = 42>```
        </cfscript>```
        y = x;
    </cfscript>"##,
    );
}

#[test]
fn cfimport_prefix_survives_across_islands() {
    let source =
        r#"```<cfimport taglib="./tags" prefix="ui">``` x = 1; ```<ui:thing value="x"/>```"#;
    let script = tag_parser::script_islands_checked(source).unwrap();
    assert!(script.contains("thing.cfm"), "{script}");
    Parser::new(script).parse().unwrap();
}

#[test]
fn malformed_islands_are_compile_errors() {
    let err = Parser::new("x=1;\n```<cfset y=2>".into())
        .parse()
        .unwrap_err();
    assert!(err.message.contains("Unclosed tag island"), "{err:?}");
    assert!(err.message.contains("line 2"), "{err:?}");
    assert!(Parser::new("```<cfif true>```".into()).parse().is_err());
    assert!(tag_parser::script_islands_checked("```<cfsavecontent variable='x'>```").is_err());
}

#[test]
fn unicode_and_crlf_are_preserved() {
    compile("component {\r\n function f() { var s='é🦀'; ```<cfoutput>日本語</cfoutput>``` return s; } }");
}

#[test]
fn stray_fence_in_a_template_does_not_hide_its_tags() {
    assert!(tag_parser::has_cfml_tags(
        "<p>use ``` for code</p>\n<cfset x = 1>"
    ));
    assert!(tag_parser::has_cfml_tags(
        "``` literal\n<cfoutput>#x#</cfoutput>"
    ));
}

#[test]
fn cr_only_line_comment_does_not_swallow_the_script_end() {
    compile("<cfscript>\r// a comment ending in CR only\rx = 1;\r</cfscript>");
    compile("x = 1;\r// comment\r```<cfset y = 2>```\r");
}

#[test]
fn quotes_inside_an_interpolated_expression_do_not_close_the_string() {
    // GH #257 shape: the same quote character nested inside #...#.
    compile(
        r##"<cfscript>
        value = 'a"b';
        result = '"#replaceNoCase( value, '"', '""', 'all' )#"';
        label = "issue ##257";
    </cfscript>"##,
    );
    compile(
        r##"function f() { var s = "#g("```")#"; ```<cfset local.x = "#g("y")#">``` return s; }"##,
    );
}

#[test]
fn a_literal_hash_in_plain_html_inside_an_island_is_text() {
    compile(r##"function f() { ```<a href="#">top</a><cfset local.x = 1>``` return x; }"##);
}
