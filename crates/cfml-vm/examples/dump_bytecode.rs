//! Print every function's bytecode for a .cfm/.cfc: `cargo run -p cfml-vm --example dump_bytecode -- file`
fn main() {
    let path = std::env::args().nth(1).expect("path");
    let src = std::fs::read_to_string(&path).expect("read");
    let conv = if cfml_compiler::tag_parser::has_cfml_tags(&src) || path.ends_with(".cfm") {
        cfml_compiler::tag_parser::tags_to_script_checked_at(&src, &path).expect("tags")
    } else { src };
    let ast = cfml_compiler::parser::Parser::new(conv).parse().expect("parse");
    let prog = cfml_codegen::compiler::CfmlCompiler::new().compile(ast);
    for f in &prog.functions {
        println!("== {} params={:?} slots={:?}", f.name, f.params, f.slot_names);
        for (i, op) in f.instructions.iter().enumerate() { println!("{:4} {:?}", i, op); }
    }
}
