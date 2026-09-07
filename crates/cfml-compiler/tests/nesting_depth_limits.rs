//! Deeply nested untrusted source must produce an error rather than recursing
//! until the stack overflows and the process aborts.
//!
//! `expr_depth_limit.rs` covers expression nesting (PR #416). This file covers
//! the recursion paths that guard does *not* reach:
//!
//!   * statement nesting  — `if(true){...}` / `function f(){...}` and the
//!     `<cfif>`/`<cfloop>` tags that lower to them, which recurse through
//!     `parse_statement` without re-entering `parse_expression`;
//!   * tag nesting        — nested body tags recurse in `tags_to_script`,
//!     overflowing *before* the parser runs, so no parser guard can help;
//!   * unary chains       — `!!!!x` / `----x` self-recurse inside
//!     `parse_not` / `parse_unary`.
//!
//! Each case aborted the process before these guards existed.
//!
//! Every case runs on an explicit 8 MiB stack — the size of the real main
//! thread. That matters: a *debug* build's parser frames are ~20x larger than a
//! release build's (a `tags_to_script_inner` frame is ~63 KB in debug vs ~1 KB
//! in release), so on libtest's smaller default thread stack even legitimate
//! 30-deep nesting overflows. The depth limits are calibrated for the shipped
//! release binary, where the margin is ~12x.

use cfml_compiler::parser::Parser;
use cfml_compiler::tag_parser;

/// Compile `source` on a thread with the same 8 MiB stack as the real main
/// thread. Returns false if the thread died (i.e. the stack overflowed).
fn compiles_without_aborting(source: String) -> bool {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let script = tag_parser::tags_to_script(&source);
            let mut parser = Parser::new(script);
            let _ = parser.parse();
        })
        .expect("spawn compile thread")
        .join()
        .is_ok()
}

#[test]
fn deeply_nested_statements_do_not_overflow_stack() {
    for depth in [10_000usize, 100_000] {
        let src = format!(
            "<cfscript>{}x = 1;{}</cfscript>",
            "if (true) {".repeat(depth),
            "}".repeat(depth)
        );
        assert!(
            compiles_without_aborting(src),
            "nested blocks (depth {depth}) must not overflow the stack"
        );
    }
}

#[test]
fn deeply_nested_function_declarations_do_not_overflow_stack() {
    let depth = 10_000;
    let src = format!(
        "<cfscript>{}x = 1;{}</cfscript>",
        "function f() {".repeat(depth),
        "}".repeat(depth)
    );
    assert!(compiles_without_aborting(src));
}

#[test]
fn deeply_nested_tags_do_not_overflow_stack() {
    // `<cfif>` lowers to nested blocks (caught by the statement guard);
    // `<cfoutput>` recurses in the preprocessor itself (caught by its guard).
    for tag in ["cfif true", "cfoutput"] {
        let name = tag.split(' ').next().unwrap();
        let depth = 10_000;
        let src = format!(
            "{}<cfset x = 1>{}",
            format!("<{tag}>").repeat(depth),
            format!("</{name}>").repeat(depth)
        );
        assert!(
            compiles_without_aborting(src),
            "nested <{name}> (depth {depth}) must not overflow the stack"
        );
    }
}

#[test]
fn long_unary_chains_do_not_overflow_stack() {
    for op in ["!", "-", "+"] {
        let src = format!("<cfset x = {}1>", op.repeat(100_000));
        assert!(
            compiles_without_aborting(src),
            "unary chain of `{op}` must not overflow the stack"
        );
    }
}

#[test]
fn an_over_deep_tag_nest_is_a_compile_error_not_silent_truncation() {
    // The preprocessor must *report* the over-deep nest, so the compile path
    // surfaces it rather than silently emitting a truncated body.
    let reported = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            let src = format!(
                "{}<cfset x = 1>{}",
                "<cfoutput>".repeat(500),
                "</cfoutput>".repeat(500)
            );
            tag_parser::tags_to_script_checked(&src).is_err()
        })
        .expect("spawn preprocess thread")
        .join()
        .unwrap_or(false);
    assert!(reported, "an over-deep tag nest must be reported as an error");
}

/// Parse `source` on the same 8 MiB stack as the real main thread, returning
/// whether it parsed cleanly (false also if the thread died).
fn parses_ok(source: String) -> bool {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut parser = Parser::new(tag_parser::tags_to_script(&source));
            parser.parse().is_ok()
        })
        .expect("spawn parse thread")
        .join()
        .unwrap_or(false)
}

#[test]
fn realistic_nesting_still_compiles() {
    // Deepest statement nesting across 2985 real files (Preside + tests/) is
    // 11 and deepest tag nesting is 4; these stay well inside the limits.
    let src = format!(
        "<cfscript>{}x = 1;{}</cfscript>",
        "if (true) {".repeat(30),
        "}".repeat(30)
    );
    assert!(parses_ok(src), "30-deep blocks must still parse");

    let tags = format!(
        "{}<cfset x = 1>{}",
        "<cfoutput>".repeat(20),
        "</cfoutput>".repeat(20)
    );
    assert!(
        tag_parser::tags_to_script_checked(&tags).is_ok(),
        "20-deep tag nesting must still preprocess"
    );

    // A realistic mix of nested control flow and expressions.
    let real = r#"
        <cfif isDefined("url.id")>
            <cfoutput>
            <cfloop from="1" to="10" index="i">
                <cfif (i mod 2) eq 0 and !(i gt 8)>
                    <cfset x = ((i + 1) * 2) - (3 / (i + 1))>
                </cfif>
            </cfloop>
            </cfoutput>
        </cfif>
    "#;
    assert!(parses_ok(real.to_string()), "realistic nested CFML must parse");
}
