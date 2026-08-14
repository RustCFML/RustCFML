//! Static JIT-coverage forecast over a CFML codebase.
//!
//! Walks a directory tree, compiles every `.cfm`/`.cfc` through the same
//! tag-preprocess → parse → codegen pipeline the VM uses, and aggregates the
//! per-op / per-function admissibility classification from
//! `cfml_vm::jit::coverage`. No code is executed.
//!
//! Usage: cargo run -p cfml-vm --example jit_coverage_scan -- <root> [<root>...]

use cfml_vm::jit::coverage::{classify, OpClass};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn collect_files(root: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name == "node_modules" || name == ".git" {
                    continue;
                }
                collect_files(&p, out);
            } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                let ext = ext.to_ascii_lowercase();
                if ext == "cfm" || ext == "cfc" {
                    out.push(p);
                }
            }
        }
    }
}

fn op_label(op: &cfml_codegen::BytecodeOp) -> String {
    let d = format!("{:?}", op);
    d.split(['(', ' ', '{']).next().unwrap_or("?").to_string()
}

fn main() {
    let roots: Vec<String> = std::env::args().skip(1).collect();
    if roots.is_empty() {
        eprintln!("usage: jit_coverage_scan <root> [<root>...]");
        std::process::exit(2);
    }
    let mut files = Vec::new();
    for r in &roots {
        collect_files(Path::new(r), &mut files);
    }
    eprintln!("scanning {} files ...", files.len());

    let mut total_fns = 0usize;
    let mut all_supported = 0usize;
    let mut boxed_admissible = 0usize;
    let mut hopeless_fns = 0usize;
    let mut total_ops = 0usize;
    let mut supported_ops = 0usize;
    let mut boxed_ops = 0usize;
    let mut hopeless_ops = 0usize;
    let mut blocking: BTreeMap<String, usize> = BTreeMap::new();
    // ops inside hopeless functions, weighted — proxy for "interpreter-stuck work"
    let mut ops_in_hopeless = 0usize;
    let mut parse_fail = 0usize;
    // For the what-if curve: per hopeless function, its distinct hopeless-op
    // label set + its total op count.
    let mut hopeless_profiles: Vec<(Vec<String>, usize)> = Vec::new();

    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let lower = f.to_string_lossy().to_lowercase();
        let needs_tags = cfml_compiler::tag_parser::has_cfml_tags(&src) || lower.ends_with(".cfm");
        let script = if needs_tags {
            match cfml_compiler::tag_parser::tags_to_script_checked(&src) {
                Ok(s) => s,
                Err(_) => {
                    parse_fail += 1;
                    continue;
                }
            }
        } else {
            src
        };
        let ast = match cfml_compiler::parser::Parser::new(script).parse() {
            Ok(a) => a,
            Err(_) => {
                parse_fail += 1;
                continue;
            }
        };
        let program = cfml_codegen::compiler::CfmlCompiler::new()
            .with_source_file(Some(f.to_string_lossy().to_string()))
            .compile(ast);
        for func in &program.functions {
            total_fns += 1;
            let mut has_hopeless = false;
            let mut has_boxed = false;
            let n_ops = func.instructions.len();
            let mut fn_hopeless: Vec<String> = Vec::new();
            for op in &func.instructions {
                total_ops += 1;
                match classify(op) {
                    OpClass::Supported => supported_ops += 1,
                    OpClass::BoxedPromising => {
                        boxed_ops += 1;
                        has_boxed = true;
                        *blocking.entry(op_label(op)).or_insert(0) += 1;
                    }
                    OpClass::Hopeless => {
                        hopeless_ops += 1;
                        has_hopeless = true;
                        let l = op_label(op);
                        if !fn_hopeless.contains(&l) {
                            fn_hopeless.push(l.clone());
                        }
                        *blocking.entry(l).or_insert(0) += 1;
                    }
                }
            }
            if has_hopeless {
                hopeless_fns += 1;
                ops_in_hopeless += n_ops;
                hopeless_profiles.push((fn_hopeless, n_ops));
            } else if has_boxed {
                boxed_admissible += 1;
            } else {
                all_supported += 1;
            }
        }
    }

    println!("=== aggregate JIT coverage forecast ===");
    println!("files scanned:        {} ({} failed to parse)", files.len(), parse_fail);
    println!("functions:            {}", total_fns);
    println!(
        "  all-supported (JIT today):        {} ({:.1}%)",
        all_supported,
        100.0 * all_supported as f64 / total_fns.max(1) as f64
    );
    println!(
        "  boxed-admissible (Option-γ):      {} ({:.1}%)",
        boxed_admissible,
        100.0 * boxed_admissible as f64 / total_fns.max(1) as f64
    );
    println!(
        "  hopeless (stays interpreter):     {} ({:.1}%)",
        hopeless_fns,
        100.0 * hopeless_fns as f64 / total_fns.max(1) as f64
    );
    println!(
        "ops: {} total — supported {} ({:.1}%), boxed-promising {} ({:.1}%), hopeless {} ({:.1}%)",
        total_ops,
        supported_ops,
        100.0 * supported_ops as f64 / total_ops.max(1) as f64,
        boxed_ops,
        100.0 * boxed_ops as f64 / total_ops.max(1) as f64,
        hopeless_ops,
        100.0 * hopeless_ops as f64 / total_ops.max(1) as f64
    );
    println!(
        "ops living inside hopeless functions: {} ({:.1}% of all ops)",
        ops_in_hopeless,
        100.0 * ops_in_hopeless as f64 / total_ops.max(1) as f64
    );
    println!("top 25 blocking ops (boxed-promising + hopeless):");
    let mut v: Vec<(String, usize)> = blocking.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, n) in v.into_iter().take(25) {
        println!("  {:>9}  {}", n, name);
    }

    // ── What-if curve: greedily add support for the hopeless op that flips
    // the most op-weight out of the hopeless bucket, cumulatively.
    println!();
    println!("=== what-if: cumulative unlock curve (greedy, op-weighted) ===");
    println!("(assumes Option-γ boxed widening is COMPLETE; each step adds one more hopeless op to the supported set)");
    let mut supported_set: Vec<String> = Vec::new();
    let base_admissible_ops: usize = total_ops - ops_in_hopeless;
    println!(
        "  step 0 (full Option-γ, no hopeless ops): {} fns / {:.1}% of ops admissible",
        total_fns - hopeless_fns,
        100.0 * base_admissible_ops as f64 / total_ops.max(1) as f64
    );
    for step in 1..=15 {
        // candidate = op whose addition unlocks max op-weight
        let mut best: Option<(String, usize, usize)> = None;
        let all_labels: std::collections::BTreeSet<String> = hopeless_profiles
            .iter()
            .flat_map(|(l, _)| l.iter().cloned())
            .filter(|l| !supported_set.contains(l))
            .collect();
        for cand in all_labels {
            let mut fns = 0usize;
            let mut ops = 0usize;
            for (labels, n_ops) in &hopeless_profiles {
                if labels
                    .iter()
                    .all(|l| l == &cand || supported_set.contains(l))
                {
                    fns += 1;
                    ops += n_ops;
                }
            }
            if best.as_ref().map_or(true, |(_, _, bo)| ops > *bo) {
                best = Some((cand, fns, ops));
            }
        }
        let Some((cand, _, _)) = best else { break };
        supported_set.push(cand.clone());
        // recompute cumulative
        let mut fns = 0usize;
        let mut ops = 0usize;
        for (labels, n_ops) in &hopeless_profiles {
            if labels.iter().all(|l| supported_set.contains(l)) {
                fns += 1;
                ops += n_ops;
            }
        }
        println!(
            "  step {:>2} +{:<22} → +{} fns, {:.1}% of all ops admissible",
            step,
            cand,
            fns,
            100.0 * (base_admissible_ops + ops) as f64 / total_ops.max(1) as f64
        );
    }
}
