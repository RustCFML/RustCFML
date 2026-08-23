# LSP Server for RustCFML — Plan

Status: Draft · Owner: RustCFML
Target: standalone LSP server for CFML/CFScript with feature parity-or-better vs
Lucee's experimental LSP, plus a VS Code client extension.

## 1. Context & baseline

### What Lucee offers (the bar to clear)
Lucee's LSP (docs.lucee.org/recipes/language-server.html, introduced 6.1) is
**experimental**. It is not a standalone tool:
- Runs as a daemon thread **inside a live Lucee server** (JVM required).
- Listens on **TCP 2089** only (no stdio).
- Delegates each JSON LSP message to a CFML component
  (`org.lucee.cfml.lsp.LSPEndpoint.cfc`); the documented default endpoint just
  echoes the JSON back.
- Every request round-trips through CFML execution at runtime.

### Why RustCFML is well positioned
The hard parts of an LSP mostly already exist:
- **Positions everywhere**: `Position`/`SourceLocation`
  (`crates/cfml-common/src/position.rs`); `TokenWithLoc` carries locations
  (`crates/cfml-compiler/src/lexer.rs:23`); every AST node carries
  `SourceLocation` (`crates/cfml-compiler/src/ast.rs`);
  `ParseError { message, line, column }` (`crates/cfml-compiler/src/parser.rs:17`).
- **Full front end**: tag preprocessor → lexer → recursive-descent parser → typed
  AST, with line numbers preserved through tag→script rewrites
  (`crates/cfml-compiler/src/tag_parser.rs:354`).
- **Semantic machinery to mine**: runtime scope resolution, component/mapping path
  resolution (`crates/cfml-vm/src/lib.rs:26887`), inheritance, native classes.
- **Infra**: tokio, serde, serde_json already in the workspace; existing
  subcommand pattern in the CLI crate (`rustcfml --serve`).

## 2. Decisions (agreed)

| Decision | Choice |
|---|---|
| Protocol layer | **tower-lsp** crate (async, full LSP 3.17 incl. semantic tokens/inlay hints) |
| Transport | **stdio only** (universal: VS Code, Neovim, Zed, JetBrains) |
| V1 feature scope | **Everything incl. rename + formatting** (largest scope) |
| Builtin signatures | **Generate a checked-in table from cfdocs.org** with a CI staleness test |
| VS Code client | Separate `vscode-rustcfml` extension spawning `rustcfml --lsp` |

Note: **tree-sitter-cfml does NOT help source builtin signatures** — it is a
syntax grammar (highlighting/indents/injections), not a function-reference
source. It IS useful for the formatter's concrete syntax tree (operates on
original source text, preserving whitespace/comments) and as a fallback
highlighting grammar for editors without semantic-token support.

## 3. Work breakdown

### Phase 0 — Crate & transport
- New crate `crates/cfml-lsp/` (lib) + `--lsp` subcommand in the CLI crate.
- tower-lsp over stdio, async tokio runtime.
- `initialize`/`shutdown`/`exit` lifecycle; server capabilities advertising the
  supported feature set (version-gated).

### Phase 1 — Compiler hardening for IDE use
Today `Parser::parse()` returns `Result<Program, ParseError>` and **stops at the
first error** (`crates/cfml-compiler/src/parser.rs:35`). Required:
- Error-recovery mode: collect **all** parse errors + lexer errors (unterminated
  strings, unterminated comments, etc.); synchronize to the next statement
  boundary; return a usable partial AST.
- Gate behind a flag/mode so the runtime path and `tests/runner.cfm` never
  regress. The repo verification gate applies: `cargo test --workspace`,
  `cargo run -- tests/runner.cfm`, serve-mode (dev + production), and the
  wasm32 build must stay green.
- Optionally expose a `parse_for_ide(source) -> (Program, Vec<Diagnostic>)` API.

### Phase 2 — Semantic analysis (new crate, e.g. `cfml-analysis`)
AST-walking binder producing a symbol table:
- **Scopes**: template `variables`; function `local`/`arguments`/params;
  closures; component `this`/`static`; properties.
- **Resolution**: identifiers → declarations; calls → builtin/user function;
  member chains (`q.col`, `obj.method()`); constructors (`new Foo()`,
  `createObject`); includes; `extends`/`implements`.
- **Cross-file index**: `.cfc` registry (path → component, functions, properties)
  using the VM's mapping logic (`resolve_path_with_mappings`,
  `crates/cfml-vm/src/lib.rs:26887`), so go-to-definition/references work across
  files.
- This is the bulk of the project and unlocks most features.

### Phase 3 — Builtin metadata
- `get_builtin_functions()` is `HashMap<String, fn>` — bare function pointers
  (`crates/cfml-stdlib/src/builtins.rs:302`). No arg names/types/required/docs.
- Generate a checked-in signature table (~400+ functions) from cfdocs.org data:
  arg names, types, required/optional, return type, summary.
- Add a CI staleness test (mirror existing hygiene gates) so the table can't
  silently drift.

### Phase 4 — LSP features
- `textDocument/publishDiagnostics` — parse + lexer diagnostics; conservative,
  opt-in warnings (undefined variables/functions — CFML dynamic scoping makes
  them unreliable; avoid false positives).
- `textDocument/completion` — keywords, scopes (local vars, functions,
  properties, builtins), member-access chains, component paths, tags in `.cfm`.
- `textDocument/signatureHelp` — builtin metadata + user-function params.
- `textDocument/hover` — type info + docs (builtins from Phase 3, user symbols
  from Phase 2).
- `textDocument/definition` + `references` — symbol table + cross-file index.
- `textDocument/documentSymbol` + `workspace/symbol` — from AST (components,
  functions, properties, vars).
- `textDocument/semanticTokens` — full classification (keywords, types,
  functions, params, properties, numbers, strings) — differentiator vs regex
  highlighters.
- `textDocument/rename` — **safe subset**: file-local vars/params/functions.
  Cross-file component renames are unsafe in a dynamic language (reflection,
  dynamic dispatch) — document the limitation.
- `textDocument/inlayHints` — parameter names.
- `textDocument/formatting` — Phase 5.

### Phase 5 — Formatter
Design wrinkle: our AST is the **preprocessed script** form (tags already
rewritten), so formatting tag-based `.cfm` from the AST is awkward.
- Options:
  (a) format from the token stream with positions mapped back to original
      source;
  (b) **use tree-sitter-cfml as the formatter's CST** — operates on original
      source text, preserving whitespace/comments (leaning option).
- Also: tree-sitter-cfml grammar/queries as a fallback highlight grammar for
  editors without semantic-token support (Neovim/Helix/Zed).

### Phase 6 — VS Code extension
- `vscode-rustcfml`: thin TypeScript client via `vscode-languageclient` spawning
  `rustcfml --lsp`; semantic-token-based highlighting; per-platform packaging or
  point at an installed binary.

### Phase 7 — Testing
Match repo culture:
- Rust unit tests on the binder/analyzer (resolution cases).
- JSON-RPC integration tests driving a server instance through
  initialize/didOpen/didChange/completion.
- Extend `tests/runner.cfm` with a suite that shells out to `rustcfml --lsp` and
  asserts LSP responses.

## 4. Comparison: Lucee vs RustCFML LSP

| Aspect | Lucee LSP | RustCFML LSP |
|---|---|---|
| Runtime | live server + JVM | standalone binary, instant cold start |
| Transport | TCP 2089 only | stdio (any editor) |
| Implementation | experimental CFC delegating JSON | typed Rust, own full parser |
| Features | completion/validation per CFC | diagnostics, completion, sig help, hover, goto-def, refs, symbols, semantic tokens, rename, formatting |
| Error recovery | — | full parse with all-errors reporting |

## 5. Risks & open decisions
1. **Error recovery must not touch the runtime path** — gate it; full verification
   gate applies (all suites incl. wasm build) since `cfml-compiler` is shared.
2. **Dynamic-language static analysis** → false-positive warnings; keep
   conservative and opt-in.
3. **Builtin table maintenance** — generator script + CI staleness test.
4. **Formatter source-preservation** — resolve original-source-vs-AST approach in
   Phase 5 (leaning tree-sitter CST).
5. **tower-lsp crate** — verify current maintenance status & WASM compatibility
   before committing (the wasm32 members must keep building).
