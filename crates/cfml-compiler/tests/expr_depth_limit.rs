//! A deeply nested expression must return a parse error rather than recursing
//! until the stack overflows and the process aborts. `Parser::parse` runs on
//! untrusted CFML source, so unbounded expression recursion is a denial of
//! service (a few hundred bytes of `((((...))))` crashes the host).

use cfml_compiler::parser::Parser;

fn parse_on_stack(source: String) -> bool {
    // 8 MiB matches the default main-thread stack. Without a depth bound this
    // input overflows and aborts; with the bound it returns Err and joins ok.
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut p = Parser::new(source);
            let _ = p.parse();
        })
        .expect("spawn parse thread")
        .join()
        .is_ok()
}

#[test]
fn deeply_nested_expression_does_not_overflow_stack() {
    for depth in [10_000usize, 100_000] {
        let src = format!("<cfset x = {}1{}>", "(".repeat(depth), ")".repeat(depth));
        assert!(
            parse_on_stack(src),
            "deeply nested expression (depth {depth}) must not overflow the stack"
        );
    }
}

#[test]
fn normal_expressions_still_parse() {
    // A realistic nested expression, well under the limit, must still parse.
    let mut p = Parser::new("<cfset x = (1 + 2) * (3 - 4) / (5 + 6)>".to_string());
    assert!(p.parse().is_ok());

    let mut p2 = Parser::new(format!("<cfset y = {}1{}>", "(".repeat(30), ")".repeat(30)));
    assert!(p2.parse().is_ok(), "30-deep nesting is under the limit");
}
