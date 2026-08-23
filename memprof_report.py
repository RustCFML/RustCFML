#!/usr/bin/env python3
"""
Roll a memprofile collapsed-stack file up into subsystem categories.

Usage:
    ./memprof_report.py rustcfml-memprofile-1-inuse.folded [--top N]

Input is the `.folded` written by `--memprofile`: one line per stack,
`root;...;leaf <bytes>`.

Two rollups are produced, because they answer different questions:

  OWNER   — scanning root->leaf, the first frame matching a rule. "Which
            subsystem asked for this memory." This is the one that answers
            'bytecode vs cached data vs live runtime state'.
  SHAPE   — scanning leaf->root, the first frame matching a rule. "What kind
            of object this is" (String, IndexMap, Vec, Arc...).

Unclassified bytes are always reported explicitly rather than folded into an
'other' bucket and forgotten — if that share is large the rules below are
wrong for this workload and should be extended.
"""
import sys
import re
from collections import defaultdict

# Ordered (category, regex) — first match wins. Ordering matters: more specific
# subsystems must precede the generic containers they are built out of.
# Scanned LEAF->ROOT, first match wins, so the most *proximate* subsystem to the
# allocation is credited. Scanning root->leaf instead would credit whatever
# generic entry point sits at the bottom of the stack (`compile_and_run`,
# `execute_function_with_args`) and swallow everything into one bucket.
# Generic containers (Vec/String/IndexMap/hashbrown) are deliberately absent so
# they fall through to the subsystem that asked for them.
OWNER_RULES = [
    ("bytecode/codegen",        r"cfml_codegen|BytecodeOp|Compiler::|compile_function|compile_program|compile_statement|compile_expr"),
    ("parse/AST",               r"cfml_compiler::(parser|lexer|ast|tag_parser)|Parser::|Lexer::|tags_to_script|parse_"),
    ("template/bytecode cache", r"bytecode_cache|template_cache|freshness|CachedTemplate|compiled_cache"),
    ("component model",         r"Component|Instance::|resolve_inheritance|component_path|method_table|CfmlFunction|attach_native_parent"),
    ("application scope",       r"application_scope|app_scope|ApplicationState|app_state"),
    ("session scope",           r"session_scope|SessionState|session::|SessionStore"),
    ("query/database",          r"mysql|sqlx|postgres|tiberius|rusqlite|CfmlQuery|query_cache|QueryResult"),
    ("cache providers",         r"cfml_stdlib::cache|memcache|CacheProvider|ehcache"),
    ("QoQ",                     r"cfml_qoq"),
    ("JIT",                     r"cranelift|jit::"),
    ("regex",                   r"regex|aho_corasick"),
    ("VFS/filesystem",          r"vfs::|std::fs::|canonicalize|read_to_string|DirEntry|read_dir"),
    ("logging",                 r"env_logger|cflog|log::__"),
    ("HTTP/server",             r"axum|hyper|tower|h2::|socketioxide|tungstenite|multer"),
    ("async runtime",           r"tokio|mio::|futures"),
    ("stdlib BIFs",             r"cfml_stdlib"),
    # Catch-all for engine work, LAST so anything more specific wins first.
    ("VM execution (other)",    r"cfml_vm|CallFrame|call_function|push_scope"),
    ("CLI/bootstrap",           r"rustcfml_cli"),
]

SHAPE_RULES = [
    ("String",       r"alloc::string::String|str::to_owned|ToOwned.*str|String::from"),
    ("Vec",          r"alloc::vec::Vec|RawVec"),
    ("IndexMap",     r"indexmap|IndexMap"),
    ("HashMap",      r"hashbrown|HashMap"),
    ("Arc/Rc",       r"alloc::sync::Arc|alloc::rc::Rc|Arc::new"),
    ("Box",          r"alloc::boxed::Box"),
    ("BTreeMap",     r"BTreeMap|btree"),
]


def classify(frames, rules, reverse=False):
    seq = reversed(frames) if reverse else frames
    for f in seq:
        for name, pat in rules:
            if re.search(pat, f):
                return name
    return None


def human(b):
    v = float(b)
    for u in ("B", "KiB", "MiB", "GiB", "TiB"):
        if v < 1024 or u == "TiB":
            return f"{v:.1f} {u}"
        v /= 1024


def main(path, top=30):
    stacks = []
    total = 0
    with open(path) as fh:
        for line in fh:
            line = line.rstrip("\n")
            if not line:
                continue
            sp = line.rfind(" ")
            if sp < 0:
                continue
            try:
                val = int(line[sp + 1:])
            except ValueError:
                continue
            frames = line[:sp].split(";")
            stacks.append((frames, val))
            total += val

    if total == 0:
        print("no samples in", path)
        return

    owner = defaultdict(int)
    shape = defaultdict(int)
    cum = defaultdict(int)
    leaf = defaultdict(int)
    unclassified_examples = []

    for frames, val in stacks:
        o = classify(frames, OWNER_RULES, reverse=True)
        if o is None:
            o = "UNCLASSIFIED"
            if len(unclassified_examples) < 12:
                unclassified_examples.append((val, frames))
        owner[o] += val

        s = classify(frames, SHAPE_RULES, reverse=True) or "UNCLASSIFIED"
        shape[s] += val

        leaf[frames[-1]] += val
        for f in set(frames):
            cum[f] += val

    print(f"file:  {path}")
    print(f"total: {human(total)}  across {len(stacks)} distinct stacks")

    def table(title, d, note=""):
        print()
        print("=" * 78)
        print(title)
        if note:
            print(note)
        print("=" * 78)
        for k, v in sorted(d.items(), key=lambda x: -x[1]):
            pct = 100.0 * v / total
            bar = "#" * int(pct / 2)
            print(f"{human(v):>11}  {pct:6.2f}%  {k:<26} {bar}")

    table("BY OWNER — which subsystem asked for the memory",
          owner,
          "(root->leaf, first rule match)")
    table("BY SHAPE — what kind of allocation it is",
          shape,
          "(leaf->root, first rule match)")

    print()
    print("=" * 78)
    print(f"TOP {top} BY CUMULATIVE FRAME — dominant retention paths")
    print("=" * 78)
    for k, v in sorted(cum.items(), key=lambda x: -x[1])[:top]:
        print(f"{human(v):>11}  {100.0*v/total:6.2f}%  {k}")

    print()
    print("=" * 78)
    print(f"TOP {top} BY LEAF FRAME — the actual allocation sites")
    print("=" * 78)
    for k, v in sorted(leaf.items(), key=lambda x: -x[1])[:top]:
        print(f"{human(v):>11}  {100.0*v/total:6.2f}%  {k}")

    if unclassified_examples:
        print()
        print("=" * 78)
        print("UNCLASSIFIED SAMPLES (largest first) — extend OWNER_RULES if this share is big")
        print("=" * 78)
        for val, frames in sorted(unclassified_examples, key=lambda x: -x[0])[:8]:
            print(f"\n  {human(val)}:")
            for f in frames[-14:]:
                print(f"      {f}")


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    topn = 30
    for a in sys.argv[1:]:
        if a.startswith("--top"):
            topn = int(a.split("=", 1)[1]) if "=" in a else 30
    main(args[0], topn)
