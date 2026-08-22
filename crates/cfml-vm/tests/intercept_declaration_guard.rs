//! The closing mechanism for `call_function`'s intercept chain.
//!
//! `call_function` is ~7,500 lines because the documented way to add a VM-intercepted
//! builtin was "append a `name_lower == \"...\"` branch". Nothing ever removed one, and
//! nothing ever recorded what the set contained — so "is this name intercepted?" could
//! only be answered by reading the whole function. That blocked compile-time BIF binding,
//! because a wrong answer does not run slow, it **silently bypasses the sandbox**.
//!
//! `cfml_common::builtins_meta::VM_INTERCEPTED` declares the set. This test proves the
//! declaration is COMPLETE by scanning the source: every name the chain compares against
//! must be declared. Append an intercept without declaring it and the build fails.
//!
//! Over-declaring is deliberately allowed — an extra entry only costs an optimisation,
//! whereas a missing one is a correctness/security bug.

use std::collections::BTreeSet;

/// Pull every literal the VM compares against `name_lower`, in all three syntactic forms.
///
/// Handling all three matters. A first attempt cut each scan window at the first `{`,
/// which silently dropped every `match name_lower { "a" => ... }` arm and found 76
/// literals instead of ~200 — precisely the quiet under-detection this guard exists to
/// prevent. The `found.len()` floor assertion below is what caught that.
fn scan_intercept_literals(src: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();

    let harvest = |text: &str, out: &mut BTreeSet<String>| {
        let mut rest = text;
        while let Some(q) = rest.find('"') {
            let after = &rest[q + 1..];
            let Some(e) = after.find('"') else { break };
            let lit = &after[..e];
            // `#[cfg(feature = "observability")]` sits inside these blocks; its string is
            // a Cargo feature, not a builtin name.
            let is_cfg_feature = rest[..q].trim_end().ends_with("feature =");
            if !is_cfg_feature
                && !lit.is_empty()
                && lit.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '$'
                })
            {
                out.insert(lit.to_string());
            }
            rest = &after[e + 1..];
        }
    };

    for (i, _) in src.match_indices("name_lower") {
        let win = &src[i..(i + 600).min(src.len())];
        let after_marker = &win["name_lower".len()..];
        let is_match_stmt = after_marker.trim_start().starts_with(".as_str() {")
            || after_marker.trim_start().starts_with('{');

        if is_match_stmt {
            // `match name_lower { "a" | "b" | … => …, }` — harvest arm HEADS until brace
            // depth returns to zero.
            //
            // The window must be large and the arm head must be taken in FULL. A first
            // version used a 600-char window and kept only the last 200 chars of each
            // head; `resolve_file_bif_paths`'s first arm is ~380 chars of `|`-separated
            // names, so it silently truncated away `fileread`, `fileexists`,
            // `directorydelete` and friends — and those are exactly the sandbox-relevant
            // ones. Truncation here is a security hole, so err large.
            let win = &src[i..(i + 8000).min(src.len())];
            let open = win.find('{').unwrap_or(0);
            let mut depth = 0i32;
            let mut end = win.len();
            for (j, c) in win[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            // Harvest EVERY quoted lowercase literal in the block, arm bodies included.
            //
            // Three attempts at precisely isolating arm heads all truncated silently:
            // a fixed 200-char tail dropped `fileread`/`fileexists` from a ~380-char arm,
            // and cutting at the last `,`/`}`/`{` dropped `arraysort` because its guard
            // clause `if matches!(args.get(1), …)` contains a comma. Each miss was a real
            // regression.
            //
            // So: stop trying to be precise. The safety asymmetry licenses this — an
            // over-collected name merely loses compile-time binding (slower, correct),
            // while a missed one loses interception (wrong, and for a filesystem builtin,
            // a sandbox hole). A simple rule that cannot truncate beats a clever one that
            // keeps doing so.
            harvest(&win[open..end], &mut found);
        } else {
            // `name_lower == "x"` and `matches!(name_lower.as_str(), "a" | "b")`.
            let stop = win.find('{').or(win.find(';')).unwrap_or(win.len());
            harvest(&win[..stop], &mut found);
        }
    }
    found
}

/// Every VM source file that may dispatch on a builtin name.
///
/// Read from disk rather than `include_str!` so that a NEW `intercepts_*.rs` is covered
/// automatically. P2 moved intercept code out of `lib.rs` into such modules, and a
/// lib.rs-only scanner stopped seeing it — silently weakening this guard at exactly the
/// moment the code was being moved. Discovery closes that hole.
fn sources() -> Vec<(String, String)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut out = vec![(
        "lib.rs".to_string(),
        std::fs::read_to_string(format!("{dir}/lib.rs")).expect("read lib.rs"),
    )];
    let mut mods: Vec<_> = std::fs::read_dir(dir)
        .expect("read src dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("intercepts_") && n.ends_with(".rs"))
        .collect();
    mods.sort();
    for m in mods {
        let body = std::fs::read_to_string(format!("{dir}/{m}")).expect("read module");
        out.push((m, body));
    }
    out
}

#[test]
fn every_intercepted_name_is_declared() {
    let files = sources();
    assert!(
        files.len() > 1,
        "no intercepts_*.rs modules found — P2 extracted several; if they were renamed, \
         update `sources()` rather than dropping the coverage"
    );
    let src: String = files.iter().map(|(_, b)| b.as_str()).collect::<Vec<_>>().join("\n");
    let found = scan_intercept_literals(&src);
    // Tripwire only: catches the scanner silently ceasing to match the source (it has
    // already done that once, dropping to 24). It is NOT a claim about the true number of
    // intercepts — an earlier regex put it at 207 by also sweeping match-ARM BODIES, which
    // are not dispatch names. 91 at time of writing; the floor sits below that so ordinary
    // churn does not trip it. If this fires, FIX THE SCANNER — do not lower the number.
    assert!(
        found.len() >= 80,
        "scanner found only {} literals (expected ~91) — it has stopped matching the \
         source. Fix the SCANNER; do not lower this floor. It is the only thing keeping \
         the intercept declaration honest.",
        found.len()
    );

    let undeclared: Vec<&String> = found
        .iter()
        .filter(|n| !cfml_common::builtins_meta::is_vm_intercepted(n))
        .collect();

    assert!(
        undeclared.is_empty(),
        "call_function dispatches on {} name(s) that are NOT declared in \
         cfml_common::builtins_meta::VM_INTERCEPTED:\n  {:?}\n\n\
         Add them to that list. An undeclared intercept is silently skipped by \
         compile-time builtin binding — for a sandbox or filesystem builtin that is a \
         security bug, not a slow path.",
        undeclared.len(),
        undeclared
    );
}
