fn main() {
    let p = std::env::args().nth(1).expect("path");
    let src = std::fs::read_to_string(p).unwrap();
    println!("{}", cfml_compiler::tag_parser::tags_to_script(&src));
}
