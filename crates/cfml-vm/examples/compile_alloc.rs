//! Allocation + time per compile stage for the given .cfc/.cfm files.
//! `cargo run --release -p cfml-vm --example compile_alloc -- <files...>`
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

struct Counting;
static BYTES: AtomicU64 = AtomicU64::new(0);
static COUNT: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        BYTES.fetch_add(l.size() as u64, Relaxed);
        COUNT.fetch_add(1, Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { System.dealloc(p, l) }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if n > l.size() { BYTES.fetch_add((n - l.size()) as u64, Relaxed); }
        COUNT.fetch_add(1, Relaxed);
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static A: Counting = Counting;

fn snap() -> (u64, u64, std::time::Instant) { (BYTES.load(Relaxed), COUNT.load(Relaxed), std::time::Instant::now()) }
fn d(a: (u64, u64, std::time::Instant)) -> String {
    let b = snap();
    format!("{:>8.1} KB {:>7} allocs {:>6.2} ms", (b.0 - a.0) as f64 / 1024.0, b.1 - a.1, a.2.elapsed().as_secs_f64() * 1000.0)
}

fn main() {
    for path in std::env::args().skip(1) {
        let src = std::fs::read_to_string(&path).expect("read");
        let n = src.len();
        let s = snap();
        let converted = if cfml_compiler::tag_parser::has_cfml_tags(&src) || path.ends_with(".cfm") {
            cfml_compiler::tag_parser::tags_to_script_checked_at(&src, &path).expect("tags")
        } else { src.clone() };
        let t_tags = d(s);
        let s = snap();
        let toks = cfml_compiler::lexer::Lexer::new(converted.clone()).tokenize();
        let t_lex = d(s);
        let ntok = toks.len();
        drop(toks);
        let s = snap();
        let ast = cfml_compiler::parser::Parser::new(converted).parse().expect("parse");
        let t_parse = d(s);
        let s = snap();
        let program = cfml_codegen::compiler::CfmlCompiler::new().with_source_file(Some(path.clone())).compile(ast);
        let t_cg = d(s);
        let s = snap();
        let _c = program.clone();
        let t_clone = d(s);
        println!("{}  ({} B source, {} tokens, TokenWithLoc={} B)\n  tags    {}\n  lex     {}\n  parse*  {}\n  codegen {}\n  clone   {}",
            path.rsplit('/').next().unwrap(), n, ntok, std::mem::size_of::<cfml_compiler::lexer::TokenWithLoc>(), t_tags, t_lex, t_parse, t_cg, t_clone);
    }
}
