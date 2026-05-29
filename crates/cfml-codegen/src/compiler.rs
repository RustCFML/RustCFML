//! CFML Code Generator - AST to bytecode

use cfml_compiler::ast::*;
use std::sync::Arc;

/// CFML built-in scope names that resolve through the VM's scope chain
/// rather than the locals map. `<name>.foo` for any of these should NOT
/// route through the LoadLocalProperty peephole, because the VM would
/// miss the fallback lookups (globals, __variables, etc.).
fn is_reserved_scope_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "variables"
            | "local"
            | "arguments"
            | "this"
            | "super"
            | "request"
            | "application"
            | "server"
            | "session"
            | "cgi"
            | "url"
            | "form"
            | "cookie"
            | "client"
            | "thread"
            | "cfthread"
            | "attributes"
            | "caller"
            | "flash"
            | "thistag"
    )
}

fn int_lit(e: &Expression) -> Option<i64> {
    if let Expression::Literal(lit) = e {
        if let LiteralValue::Int(n) = lit.value {
            return Some(n);
        }
    }
    None
}

/// Helper function to capitalize the first letter of a string
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(String::new)
        + &s[1..]
}

pub struct CfmlCompiler {
    pub program: BytecodeProgram,
    /// Stack of (break_placeholder_indices, continue_placeholder_indices) for loops
    loop_stack: Vec<(Vec<usize>, Vec<usize>)>,
    /// Finally body to emit before rethrow (set when inside try-catch-finally)
    current_finally: Option<Vec<Statement>>,
    /// Nesting depth of function-body compilation. 0 means page-scope; inside any
    /// UDF or CFC method this is > 0. Used to gate the `variables.x` peephole:
    /// at page scope `variables.x` is a read of globals (LoadGlobal semantics),
    /// but inside a function body `variables` refers to the local-scope merge or
    /// a CFC's `__variables` struct — different semantics entirely.
    function_depth: usize,
    /// Declared `localMode` of the function currently being compiled. Used so
    /// that closures defined inside that function inherit its declared mode
    /// when the closure itself doesn't carry an explicit attribute. `None` =
    /// at page scope or inside a function that didn't declare its mode (the
    /// closure also inherits `None` and falls back to the application
    /// default at runtime).
    current_fn_local_mode: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct BytecodeProgram {
    pub functions: Vec<Arc<BytecodeFunction>>,
}

#[derive(Debug, Clone)]
pub struct BytecodeFunction {
    pub name: String,
    pub params: Vec<String>,
    /// Which params are required (parallel to `params`; true = required)
    pub required_params: Vec<bool>,
    pub instructions: Vec<BytecodeOp>,
    pub source_file: Option<String>,
    /// Lucee `localMode` for this function. `Some(true)` = modern (unscoped
    /// writes stay in `local`), `Some(false)` = classic (unscoped writes go
    /// to `variables`/`__variables`). `None` = inherit at runtime from the
    /// application default (`this.localMode` in Application.cfc), falling
    /// back to classic if no app default is set.
    pub declared_local_mode: Option<bool>,
}

/// Inspect a function/closure metadata attribute list for `localMode`.
/// Returns `Some(true)` for modern aliases (`modern`/`always`/`true`),
/// `Some(false)` for classic aliases (`classic`/`update`/`false`),
/// `None` if no `localMode` attribute is present, or its value is not a
/// recognised alias. `None` means "inherit at runtime" — the VM resolves it
/// against the application default (`this.localMode` in Application.cfc),
/// falling back to classic. Case-insensitive. The VM extractor in
/// `extract_app_config` uses the same alias set and `None`-on-unknown rule.
pub fn metadata_declared_local_mode(metadata: &[(String, String)]) -> Option<bool> {
    for (k, v) in metadata {
        if k.eq_ignore_ascii_case("localmode") {
            return match v.trim().to_ascii_lowercase().as_str() {
                "modern" | "always" | "true" => Some(true),
                "classic" | "update" | "false" => Some(false),
                _ => None,
            };
        }
    }
    None
}

/// Comparison operator tag for fused-compare super-instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Lte,
    Gt,
    Gte,
    Eq,
    Neq,
}

#[derive(Debug, Clone)]
pub enum BytecodeOp {
    // Literals
    Null,
    True,
    False,
    Integer(i64),
    Double(f64),
    String(String),

    // Variables
    LoadLocal(String),
    StoreLocal(String),
    /// Fused arrayAppend(<ident>, value): pops the value off the stack and
    /// appends it to the array held in the named variable, in place. Avoids the
    /// LoadLocal/clone + builtin call + StoreLocal round-trip whose Arc aliasing
    /// makes a loop of appends O(n²) (every `make_mut` deep-clones the backing
    /// Vec). Emitted only for a 2-arg call with a simple, non-scope identifier.
    ArrayAppendLocal(String),
    LoadGlobal(String),
    StoreGlobal(String),

    // Stack
    Pop,
    Dup,
    Swap,

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    IntDiv,
    Negate,

    // String
    Concat,

    // Comparison
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    Contains,
    DoesNotContain,

    // Logical
    And,
    Or,
    Not,
    Xor,
    Eqv,
    Imp,

    // Control flow
    Jump(usize),
    JumpIfFalse(usize),
    JumpIfTrue(usize),
    /// Loop-condition super-instruction: `if !(locals[name] CMP const) { jump offset }`.
    /// Fuses LoadLocal + Integer + Cmp + JumpIfFalse into one dispatch.
    /// Emitted by compile_for for conditions of the shape `<identifier> <cmp> <int-const>`.
    JumpIfLocalCmpConstFalse(String, i64, CmpOp, usize),
    /// For-loop step super-instruction: `locals[name] += step; if (locals[name] CMP const) jump target`.
    /// Fuses Increment + LoadLocal + Integer + Cmp + JumpIfFalse-style test into one
    /// dispatch. `step` is +1 (for `i++`) or -1 (for `i--`). The jump fires on the
    /// TRUE arm (back to body); falling through means the loop has finished.
    ForLoopStep(String, i64, CmpOp, i64, usize),
    Call(usize),
    Return,

    // Collections
    BuildArray(usize),   // Build array from top N stack items
    BuildStruct(usize),  // Build struct from top N key-value pairs
    GetIndex,            // Get array[index] or struct[key]
    SetIndex,            // Set array[index] = value or struct[key] = value
    GetProperty(String), // Get object.property
    /// Fused LoadLocal(name) + GetProperty(member) — reads a struct field from a
    /// named local in one dispatch. Only emitted for non-null-safe accesses
    /// where the receiver is a plain identifier (the common `s.foo` pattern).
    LoadLocalProperty(String, String),
    /// Fused LoadLocal(name) + SetProperty(member) — stores a value into a struct
    /// field of a named local in one dispatch. Only emitted for non-null-safe
    /// accesses where the receiver is a plain identifier (the common `s.foo = x` pattern).
    StoreLocalProperty(String, String),
    SetProperty(String), // Set object.property = value

    // Object
    NewObject(usize),  // arg_count for constructor

    // Function definition
    DefineFunction(usize), // Index into program.functions

    // Postfix ops
    Increment(String),  // Increment variable (+1)
    Decrement(String),  // Decrement variable (-1)
    AddLocalConst(String, i64),  // Add constant to local: i += K or i = i + K
    MulLocalConst(String, i64),  // Multiply local by constant: i *= K

    // Exception handling
    TryStart(usize),    // Jump target for catch
    TryEnd,
    Throw,
    Rethrow,            // Re-throw current exception

    // Method call: object is on stack, then args, method name + arg count
    // Optional write-back: (object_var, Option<property_name>)
    //   - Some(vec!["dog"]) for dog.method() — write modified this back to dog
    //   - Some(vec!["this", "items"]) for this.items.method() — write result back to this.items
    //   - Some(vec!["local", "_taffy", "factory"]) for local._taffy.factory.method()
    //   - None — no write-back needed
    CallMethod(String, usize, Option<Vec<String>>),
    CallMethodNamed(String, Vec<String>, usize, Option<Vec<String>>),

    // For-in support
    GetKeys,  // Pop value: if struct, push array of keys; if array, leave as-is

    // Include
    Include(String),  // Include and execute a file (static path)
    IncludeDynamic,   // Include: pop path from stack (dynamic expression)

    // Null handling
    IsNull,                // Pop value, push bool (true if Null)
    JumpIfNotNull(usize),  // Pop value, jump if not null (pushes value back)

    // Output
    Print,
    Halt,

    // Variable existence check
    IsDefined(String),

    // Spread operator support
    ConcatArrays,
    MergeStructs,
    CallSpread,

    // Source location tracking
    LineInfo(usize, usize),  // (line, column) — emitted before statements for stack traces

    // Safe variable load: returns Null for undefined vars (used by Elvis, null-safe, isNull)
    TryLoadLocal(String),

    // Declare a variable as function-local (var keyword) — prevents writeback to parent scope
    DeclareLocal(String),

    // Named function call: like Call but carries argument names for name-to-param mapping
    // (names, arg_count) — names[i] corresponds to the i-th arg on the stack
    CallNamed(Vec<String>, usize),

    // Explicit super(args) constructor call for a CFC whose parent is a Rust class.
    // Pops arg_count values, looks up the constructor registered under
    // this.__rust_extends, calls it, and stores the new NativeObject on
    // this.__super (replacing any default-constructed one). Pushes Null.
    CallRustSuperCtor(usize),
}

impl CfmlCompiler {
    pub fn new() -> Self {
        Self {
            program: BytecodeProgram {
                functions: vec![Arc::new(BytecodeFunction {
                    name: "__main__".to_string(),
                    params: Vec::new(),
                    required_params: Vec::new(),
                    instructions: Vec::new(),
                    source_file: None,
                    declared_local_mode: None,
                })],
            },
            loop_stack: Vec::new(),
            current_finally: None,
            function_depth: 0,
            current_fn_local_mode: None,
        }
    }

    /// Flatten a member-access chain like a.b.c into "a.b.c" for dotted new expressions.
    fn flatten_member_access(expr: &Expression) -> Option<String> {
        match expr {
            Expression::Identifier(ident) => Some(ident.name.clone()),
            Expression::MemberAccess(access) => {
                if let Some(base) = Self::flatten_member_access(&access.object) {
                    Some(format!("{}.{}", base, access.member))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Determine write-back target for a method call from the AST.
    /// Returns Some((var_name, Some(prop_name))) for obj.prop.method()
    /// or Some((var_name, None)) for var.method()
    fn method_call_write_back(object: &Expression) -> Option<Vec<String>> {
        // Recursively collect the member access chain: a.b.c.method()
        // returns vec!["a", "b", "c"]
        fn collect_path(expr: &Expression, path: &mut Vec<String>) -> bool {
            match expr {
                Expression::Identifier(ident) => {
                    path.push(ident.name.clone());
                    true
                }
                Expression::This(_) => {
                    path.push("this".to_string());
                    true
                }
                Expression::Super(_) => {
                    path.push("this".to_string());
                    true
                }
                Expression::MemberAccess(access) => {
                    if collect_path(&access.object, path) {
                        path.push(access.member.clone());
                        true
                    } else {
                        false
                    }
                }
                Expression::MethodCall(call) => {
                    // For chained calls like a.b().c(), extract the root path
                    // so all calls in the chain write back to the same variable.
                    // BUT: if the inner method returns a new value distinct from
                    // its receiver (filter/map/slice/etc.), the outer call is
                    // operating on that new value, not on `a` — so propagating
                    // the path would cause the outer call's result to clobber
                    // `a`. Break the chain for known transformative methods.
                    let inner_lower = call.method.to_lowercase();
                    let is_transformative = matches!(
                        inner_lower.as_str(),
                        "filter" | "map" | "slice" | "reduce" | "tolist"
                        | "toarray" | "tojson" | "serializejson" | "merge"
                        | "splice" | "indexexists" | "keyarray" | "keylist"
                        | "valuearray" | "copy"
                    );
                    if is_transformative {
                        return false;
                    }
                    collect_path(&call.object, path)
                }
                _ => false,
            }
        }

        let mut path = Vec::new();
        if collect_path(object, &mut path) {
            Some(path)
        } else {
            None
        }
    }

    pub fn compile(mut self, ast: Program) -> BytecodeProgram {
        let mut instructions = Vec::new();

        // Hoist top-level function declarations: standard CFML behaviour is
        // that <cffunction> / `function f(){}` declarations at template scope
        // are callable from anywhere in the template, regardless of textual
        // order. Compile each top-level FunctionDecl up front so its name is
        // bound before the body runs, then re-emit DefineFunction+StoreLocal
        // at the original textual position. The re-emit re-runs the
        // closure-env sync (DefineFunction folds current locals into the
        // shared closure env), preserving the snapshot semantics that
        // existing scope-capture tests rely on.
        let mut hoisted_indices: Vec<usize> = Vec::new();
        for node in &ast.statements {
            if let CfmlNode::Statement(Statement::FunctionDecl(fd)) = node {
                self.compile_function_decl(&fd.func, &mut instructions);
                // compile_function_decl ends with DefineFunction(idx) +
                // StoreLocal(name); the function's idx is the one in that
                // penultimate op (it is NOT len()-1 of program.functions
                // because nested anon-fn decls inside the body push their
                // own entries first).
                let idx = match instructions.get(instructions.len().saturating_sub(2)) {
                    Some(BytecodeOp::DefineFunction(i)) => *i,
                    _ => panic!("compile_function_decl did not end with DefineFunction"),
                };
                hoisted_indices.push(idx);
            }
        }
        let mut hoist_iter = hoisted_indices.into_iter();
        for node in &ast.statements {
            if let CfmlNode::Statement(Statement::FunctionDecl(fd)) = node {
                let idx = hoist_iter.next().expect("hoist index");
                instructions.push(BytecodeOp::DefineFunction(idx));
                instructions.push(BytecodeOp::StoreLocal(fd.func.name.clone()));
            } else {
                self.compile_node(node, &mut instructions);
            }
        }

        instructions.push(BytecodeOp::Halt);

        Arc::get_mut(&mut self.program.functions[0]).unwrap().instructions = instructions;

        self.program
    }

    fn compile_node(&mut self, node: &CfmlNode, instructions: &mut Vec<BytecodeOp>) {
        match node {
            CfmlNode::Statement(stmt) => self.compile_statement(stmt, instructions),
            CfmlNode::Expression(expr) => {
                self.compile_expression(expr, instructions);
                instructions.push(BytecodeOp::Pop);
            }
            _ => {}
        }
    }

    /// Check if an expression is a call to a known mutating function with a simple
    /// variable as the first argument. Returns the variable name for write-back.
    /// e.g. structAppend(myStruct, other) → Some("myStruct")
    fn is_mutating_standalone_call(expr: &Expression) -> bool {
        if let Expression::FunctionCall(call) = expr {
            if let Expression::Identifier(ident) = &*call.name {
                let name_lower = ident.name.to_lowercase();
                return matches!(name_lower.as_str(),
                    "structappend" | "structinsert" | "structdelete" | "structupdate" |
                    "structclear" | "arrayclear" | "arrayappend" | "arrayprepend" |
                    "arrayinsert" | "arrayinsertat" | "arraydeleteat" | "arraysort" |
                    "arrayresize" | "arrayswap" | "arrayreverse" | "arrayset" |
                    "queryaddcolumn" |
                    "querydeleterow" | "querydeletecolumn" | "querysort"
                ) && !call.arguments.is_empty();
            }
        }
        false
    }

    /// True when `expr` is exactly `arrayAppend(<ident>, value)` — a two-arg
    /// append whose first argument is the given simple identifier and which is
    /// not a reserved scope name. These compile to the fused `ArrayAppendLocal`
    /// op for an O(1) in-place append. The merge form (`arrayAppend(a, b, true)`)
    /// and member-access targets keep the generic clone+store path.
    fn is_inplace_array_append(expr: &Expression, ident: &Identifier) -> bool {
        if let Expression::FunctionCall(call) = expr {
            if let Expression::Identifier(name) = &*call.name {
                if name.name.eq_ignore_ascii_case("arrayappend")
                    && call.arguments.len() == 2
                    && !call.arguments.iter().any(|a| matches!(a, Expression::NamedArgument(_)))
                    && !Self::is_reserved_scope_name(&ident.name)
                {
                    return true;
                }
            }
        }
        false
    }

    /// Scope keywords that must never be treated as plain mutable variables.
    fn is_reserved_scope_name(name: &str) -> bool {
        matches!(name.to_lowercase().as_str(),
            "local" | "variables" | "arguments" | "this" | "super" | "request" |
            "application" | "session" | "server" | "cgi" | "url" | "form" |
            "cookie" | "client" | "thread"
        )
    }

    /// Get the first argument expression of a function call (for mutating write-back).
    fn mutating_call_first_arg(expr: &Expression) -> Option<&Expression> {
        if let Expression::FunctionCall(call) = expr {
            call.arguments.first()
        } else {
            None
        }
    }

    /// Emit write-back instructions for nested property assignment.
    /// After SetProperty("leaf"), the modified intermediate object is on the stack.
    /// This walks up the MemberAccess chain, writing back to each parent level.
    /// e.g. for `s.a.b = val`: after SetProperty("b"), stack has modified s.a
    ///   → Load s, Swap, SetProperty("a") → stack has modified s → StoreLocal("s")
    /// Emit bytecode to write back a modified nested value through the property chain.
    /// Stack state on entry: [modified_value]
    /// For `s.a.b = val`, after SetProperty("b"), stack has [modified_a_struct].
    /// We need to: load s, swap, SetProperty("a"), StoreLocal("s").
    /// For deeper chains like `s.a.b.c = val`, we recurse up the chain.
    fn emit_nested_writeback(obj: &Expression, instructions: &mut Vec<BytecodeOp>) {
        match obj {
            Expression::Identifier(ident) => {
                instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::StoreLocal("this".to_string()));
            }
            Expression::MemberAccess(access) => {
                // Stack has modified child value. Load the parent, swap, set property.
                // Then recurse to write back the parent.
                Self::emit_load_for_writeback(&access.object, instructions);
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(access.member.clone()));
                Self::emit_nested_writeback(&access.object, instructions);
            }
            Expression::ArrayAccess(access) => {
                // Stack has [modified_value]. We need to write it back into the parent collection.
                // Load the parent collection, then the index, then SetIndex, then recurse.
                // e.g. for `a.b[0][1] = val`: after inner SetIndex, stack has modified inner array.
                // We need to: load a.b[0], swap, push 0-index, SetIndex → modified a.b, then write back a.b.
                Self::emit_load_for_writeback(&access.array, instructions);
                Self::compile_expression_static(&access.index, instructions);
                // Stack: [modified_value, parent_collection, index]
                // We need: [value_to_set, collection, index] for SetIndex
                // Rearrange: rotate so modified_value goes under collection
                // Actually SetIndex wants [value, collection, index] bottom-to-top
                // Current: [modified_value, parent_collection, index]
                // That's already correct for SetIndex
                instructions.push(BytecodeOp::SetIndex);
                Self::emit_nested_writeback(&access.array, instructions);
            }
            _ => {
                instructions.push(BytecodeOp::Pop);
            }
        }
    }

    /// Emit a load instruction for the given expression (used during write-back chain).
    fn emit_load_for_writeback(expr: &Expression, instructions: &mut Vec<BytecodeOp>) {
        match expr {
            Expression::Identifier(ident) => {
                instructions.push(BytecodeOp::LoadLocal(ident.name.clone()));
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::LoadLocal("this".to_string()));
            }
            Expression::MemberAccess(access) => {
                // For nested access like loading "s.a", we load s then get property a
                Self::emit_load_for_writeback(&access.object, instructions);
                instructions.push(BytecodeOp::GetProperty(access.member.clone()));
            }
            Expression::ArrayAccess(access) => {
                // For nested access like loading "s.a[0]", we load s.a then get index 0
                Self::emit_load_for_writeback(&access.array, instructions);
                Self::compile_expression_static(&access.index, instructions);
                instructions.push(BytecodeOp::GetIndex);
            }
            _ => {
                // Can't load this expression for writeback
                instructions.push(BytecodeOp::Null);
            }
        }
    }

    /// Static helper to compile an expression into instructions (for use in static methods)
    fn compile_expression_static(expr: &Expression, instructions: &mut Vec<BytecodeOp>) {
        match expr {
            Expression::Literal(lit) => {
                match &lit.value {
                    LiteralValue::String(s) => instructions.push(BytecodeOp::String(s.clone())),
                    LiteralValue::Int(i) => instructions.push(BytecodeOp::Integer(*i)),
                    LiteralValue::Double(d) => instructions.push(BytecodeOp::Double(*d)),
                    LiteralValue::Bool(b) => instructions.push(if *b { BytecodeOp::True } else { BytecodeOp::False }),
                    LiteralValue::Null => instructions.push(BytecodeOp::Null),
                }
            }
            Expression::Identifier(ident) => {
                instructions.push(BytecodeOp::LoadLocal(ident.name.clone()));
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::LoadLocal("this".to_string()));
            }
            Expression::MemberAccess(access) => {
                Self::compile_expression_static(&access.object, instructions);
                instructions.push(BytecodeOp::GetProperty(access.member.clone()));
            }
            Expression::ArrayAccess(access) => {
                Self::compile_expression_static(&access.array, instructions);
                Self::compile_expression_static(&access.index, instructions);
                instructions.push(BytecodeOp::GetIndex);
            }
            _ => {
                // For complex expressions, emit Null as fallback
                instructions.push(BytecodeOp::Null);
            }
        }
    }

    fn stmt_line(stmt: &Statement) -> Option<usize> {
        match stmt {
            Statement::Expression(e) => Some(e.location.start.line),
            Statement::Var(v) => Some(v.location.start.line),
            Statement::Assignment(a) => Some(a.location.start.line),
            Statement::If(i) => Some(i.location.start.line),
            Statement::For(f) => Some(f.location.start.line),
            Statement::ForIn(f) => Some(f.location.start.line),
            Statement::While(w) => Some(w.location.start.line),
            Statement::Do(d) => Some(d.location.start.line),
            Statement::Switch(s) => Some(s.location.start.line),
            Statement::Return(r) => Some(r.location.start.line),
            Statement::FunctionDecl(f) => Some(f.func.location.start.line),
            Statement::Try(t) => Some(t.location.start.line),
            Statement::Throw(t) => Some(t.location.start.line),
            Statement::Rethrow(loc) => Some(loc.start.line),
            Statement::ComponentDecl(c) => Some(c.component.location.start.line),
            Statement::InterfaceDecl(i) => Some(i.interface.location.start.line),
            Statement::Include(i) => Some(i.location.start.line),
            Statement::Break(b) => Some(b.location.start.line),
            Statement::Continue(c) => Some(c.location.start.line),
            Statement::Import(i) => Some(i.location.start.line),
            Statement::Output(o) => Some(o.location.start.line),
            Statement::PropertyDecl(p) => Some(p.prop.location.start.line),
            Statement::Exit => None,
        }
    }

    fn compile_statement(&mut self, stmt: &Statement, instructions: &mut Vec<BytecodeOp>) {
        if let Some(line) = Self::stmt_line(stmt) {
            instructions.push(BytecodeOp::LineInfo(line, 0));
        }

        match stmt {
            Statement::Expression(expr_stmt) => {
                // Peephole: `i++;` / `i--;` / `++i;` / `--i;` as a bare statement.
                // The normal 5-op expand (Load/Dup/Int1/Add/Store) plus a trailing
                // Pop collapses to a single Increment/Decrement.
                if self.try_emit_inc_dec_statement(&expr_stmt.expr, instructions) {
                    // emitted; no Pop needed — the op has no stack effect
                }
                // Check for mutating function calls: structAppend(a, b), structInsert(a, k, v), etc.
                // These return the modified struct; store it back to the first arg's location.
                else if Self::is_mutating_standalone_call(&expr_stmt.expr) {
                    if let Some(first_arg) = Self::mutating_call_first_arg(&expr_stmt.expr) {
                        match first_arg {
                            Expression::Identifier(ident)
                                if Self::is_inplace_array_append(&expr_stmt.expr, ident) =>
                            {
                                // Hot path: arrayAppend(<ident>, value) with exactly two
                                // args. Push the value, then append in place via the fused
                                // op — no array clone, no StoreLocal round-trip. This turns
                                // a quadratic append loop linear.
                                if let Expression::FunctionCall(call) = &expr_stmt.expr {
                                    self.compile_expression(&call.arguments[1], instructions);
                                }
                                instructions.push(BytecodeOp::ArrayAppendLocal(ident.name.clone()));
                            }
                            Expression::Identifier(ident) => {
                                // Simple: structAppend(a, b) → compile call → StoreLocal(a)
                                self.compile_expression(&expr_stmt.expr, instructions);
                                instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
                            }
                            Expression::MemberAccess(_) => {
                                // Nested: structAppend(local._taffy.settings, defaultConfig)
                                // → compile call → emit_nested_writeback(local._taffy.settings)
                                self.compile_expression(&expr_stmt.expr, instructions);
                                Self::emit_nested_writeback(first_arg, instructions);
                            }
                            _ => {
                                // Can't write back — just pop
                                self.compile_expression(&expr_stmt.expr, instructions);
                                instructions.push(BytecodeOp::Pop);
                            }
                        }
                    } else {
                        self.compile_expression(&expr_stmt.expr, instructions);
                        instructions.push(BytecodeOp::Pop);
                    }
                } else {
                    self.compile_expression(&expr_stmt.expr, instructions);
                    instructions.push(BytecodeOp::Pop);
                }
            }
            Statement::Var(var) => {
                instructions.push(BytecodeOp::DeclareLocal(var.name.clone()));
                if let Some(value) = &var.value {
                    self.compile_expression(value, instructions);
                } else {
                    instructions.push(BytecodeOp::Null);
                }
                instructions.push(BytecodeOp::StoreLocal(var.name.clone()));
            }
            Statement::Assignment(assign) => {
                // Hot-path: x += <int>, x -= <int>, x *= <int> compile to a single
                // load-compute-store op inside locals. No stack traffic, no trailing
                // StoreLocal.
                if let AssignTarget::Variable(name) = &assign.target {
                    if let Some(k) = int_lit(&assign.value) {
                        let op = match &assign.operator {
                            AssignOp::PlusEqual  => Some(BytecodeOp::AddLocalConst(name.clone(),  k)),
                            AssignOp::MinusEqual => Some(BytecodeOp::AddLocalConst(name.clone(), -k)),
                            AssignOp::StarEqual  => Some(BytecodeOp::MulLocalConst(name.clone(),  k)),
                            _ => None,
                        };
                        if let Some(op) = op {
                            instructions.push(op);
                            return;
                        }
                    }
                }

                self.compile_expression(&assign.value, instructions);

                match &assign.operator {
                    AssignOp::PlusEqual => {
                        if let AssignTarget::Variable(name) = &assign.target {
                            instructions.push(BytecodeOp::LoadLocal(name.clone()));
                        }
                        // Reorder to [target, rhs] with a RUNTIME swap. A
                        // compile-time instruction swap would only be correct
                        // when the RHS is a single push; for multi-op RHS like
                        // `x += arr[i]` or `x -= obj.p` it corrupts the bytecode.
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Add);
                    }
                    AssignOp::MinusEqual => {
                        if let AssignTarget::Variable(name) = &assign.target {
                            instructions.push(BytecodeOp::LoadLocal(name.clone()));
                        }
                        // Reorder to [target, rhs] with a RUNTIME swap. A
                        // compile-time instruction swap would only be correct
                        // when the RHS is a single push; for multi-op RHS like
                        // `x += arr[i]` or `x -= obj.p` it corrupts the bytecode.
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Sub);
                    }
                    AssignOp::StarEqual => {
                        if let AssignTarget::Variable(name) = &assign.target {
                            instructions.push(BytecodeOp::LoadLocal(name.clone()));
                        }
                        // Reorder to [target, rhs] with a RUNTIME swap. A
                        // compile-time instruction swap would only be correct
                        // when the RHS is a single push; for multi-op RHS like
                        // `x += arr[i]` or `x -= obj.p` it corrupts the bytecode.
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Mul);
                    }
                    AssignOp::SlashEqual => {
                        match &assign.target {
                            AssignTarget::Variable(name) => {
                                instructions.push(BytecodeOp::LoadLocal(name.clone()));
                            }
                            _ => {}
                        }
                        // Reorder to [target, rhs] with a RUNTIME swap. A
                        // compile-time instruction swap would only be correct
                        // when the RHS is a single push; for multi-op RHS like
                        // `x += arr[i]` or `x -= obj.p` it corrupts the bytecode.
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Div);
                    }
                    AssignOp::PercentEqual => {
                        match &assign.target {
                            AssignTarget::Variable(name) => {
                                instructions.push(BytecodeOp::LoadLocal(name.clone()));
                            }
                            _ => {}
                        }
                        // Reorder to [target, rhs] with a RUNTIME swap. A
                        // compile-time instruction swap would only be correct
                        // when the RHS is a single push; for multi-op RHS like
                        // `x += arr[i]` or `x -= obj.p` it corrupts the bytecode.
                        instructions.push(BytecodeOp::Swap);
                        instructions.push(BytecodeOp::Mod);
                    }
                    AssignOp::ConcatEqual => {
                        match &assign.target {
                            AssignTarget::Variable(name) => {
                                instructions.push(BytecodeOp::LoadLocal(name.clone()));
                                // Runtime swap → [target, rhs]; correct even when
                                // the RHS compiled to multiple instructions.
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::Concat);
                            }
                            AssignTarget::StructAccess(obj, member) => {
                                // Stack: [rhs]. Load current, then Swap + Concat so
                                // the final stack order is [current, rhs] → [result].
                                self.compile_expression(obj, instructions);
                                instructions.push(BytecodeOp::GetProperty(member.clone()));
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::Concat);
                            }
                            AssignTarget::ArrayAccess(arr, idx) => {
                                self.compile_expression(arr, instructions);
                                self.compile_expression(idx, instructions);
                                instructions.push(BytecodeOp::GetIndex);
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::Concat);
                            }
                        }
                    }
                    AssignOp::Equal => {} // Value already on stack
                }

                match &assign.target {
                    AssignTarget::Variable(name) => {
                        instructions.push(BytecodeOp::StoreLocal(name.clone()));
                    }
                    AssignTarget::ArrayAccess(arr, idx) => {
                        self.compile_expression(arr, instructions);
                        self.compile_expression(idx, instructions);
                        instructions.push(BytecodeOp::SetIndex);
                        // SetIndex leaves modified collection on stack; write it back
                        Self::emit_nested_writeback(arr, instructions);
                    }
                    AssignTarget::StructAccess(obj, member) => {
                        if let Expression::Identifier(ref ident) = **obj {
                            if !is_reserved_scope_name(&ident.name) {
                                instructions.push(BytecodeOp::StoreLocalProperty(
                                    ident.name.clone(),
                                    member.clone(),
                                ));
                            } else {
                                self.compile_expression(obj, instructions);
                                instructions.push(BytecodeOp::Swap);
                                instructions.push(BytecodeOp::SetProperty(member.clone()));
                                Self::emit_nested_writeback(obj, instructions);
                            }
                        } else {
                            self.compile_expression(obj, instructions);
                            instructions.push(BytecodeOp::Swap);
                            instructions.push(BytecodeOp::SetProperty(member.clone()));
                            Self::emit_nested_writeback(obj, instructions);
                        }
                    }
                }
            }
            Statement::Return(ret) => {
                if let Some(value) = &ret.value {
                    self.compile_expression(value, instructions);
                } else {
                    instructions.push(BytecodeOp::Null);
                }
                instructions.push(BytecodeOp::Return);
            }
            Statement::If(if_stmt) => {
                self.compile_if(if_stmt, instructions);
            }
            Statement::For(for_stmt) => {
                self.compile_for(for_stmt, instructions);
            }
            Statement::ForIn(for_in) => {
                self.compile_for_in(for_in, instructions);
            }
            Statement::While(while_stmt) => {
                self.compile_while(while_stmt, instructions);
            }
            Statement::Do(do_stmt) => {
                self.compile_do(do_stmt, instructions);
            }
            Statement::Switch(switch_stmt) => {
                self.compile_switch(switch_stmt, instructions);
            }
            Statement::Break(_) => {
                // Push a placeholder jump that will be patched later
                let idx = instructions.len();
                instructions.push(BytecodeOp::Jump(0)); // placeholder
                if let Some(loop_ctx) = self.loop_stack.last_mut() {
                    loop_ctx.0.push(idx); // break indices
                }
            }
            Statement::Continue(_) => {
                let idx = instructions.len();
                instructions.push(BytecodeOp::Jump(0)); // placeholder
                if let Some(loop_ctx) = self.loop_stack.last_mut() {
                    loop_ctx.1.push(idx); // continue indices
                }
            }
            Statement::Try(try_stmt) => {
                self.compile_try(try_stmt, instructions);
            }
            Statement::Throw(throw_stmt) => {
                if let Some(msg) = &throw_stmt.message {
                    self.compile_expression(msg, instructions);
                } else {
                    instructions.push(BytecodeOp::String("An error occurred".to_string()));
                }
                instructions.push(BytecodeOp::Throw);
            }
            Statement::Rethrow(_loc) => {
                // Emit finally body before rethrow if we're inside a try-catch-finally
                if let Some(ref finally_body) = self.current_finally.clone() {
                    for s in finally_body {
                        self.compile_statement(s, instructions);
                    }
                }
                instructions.push(BytecodeOp::Rethrow);
            }
            Statement::FunctionDecl(func_decl) => {
                self.compile_function_decl(&func_decl.func, instructions);
            }
            Statement::ComponentDecl(comp_decl) => {
                // Compile component as a struct with methods
                self.compile_component(&comp_decl.component, instructions);
            }
            Statement::InterfaceDecl(iface_decl) => {
                self.compile_interface(&iface_decl.interface, instructions);
            }
            Statement::Include(inc) => {
                // Static path: emit Include(path) directly
                if let Expression::Literal(lit) = &inc.path {
                    if let LiteralValue::String(path) = &lit.value {
                        instructions.push(BytecodeOp::Include(path.clone()));
                        return;
                    }
                }
                // Dynamic path: compile expression, pop from stack at runtime
                self.compile_expression(&inc.path, instructions);
                instructions.push(BytecodeOp::IncludeDynamic);
            }
            Statement::Import(_) => {
                // Import not yet supported at bytecode level
            }
            Statement::Exit => {
                instructions.push(BytecodeOp::Halt);
            }
            Statement::Output(output) => {
                // Compile each statement in the output block body
                for body_stmt in &output.body {
                    self.compile_statement(body_stmt, instructions);
                }
            }
            _ => {}
        }
    }

    fn compile_if(&mut self, if_stmt: &If, instructions: &mut Vec<BytecodeOp>) {
        let jump_false_idx = self.emit_cond_jump_false(&if_stmt.condition, instructions);

        // Then branch
        for s in &if_stmt.then_branch {
            self.compile_statement(s, instructions);
        }

        if !if_stmt.else_if.is_empty() || if_stmt.else_branch.is_some() {
            let jump_end_idx = instructions.len();
            instructions.push(BytecodeOp::Jump(0)); // placeholder

            // Patch the jump-to-else
            let end_of_then = instructions.len();
            Self::patch_cond_jump_target(instructions, jump_false_idx, end_of_then);

            // Else-if chains
            let mut end_jumps = vec![jump_end_idx];

            for (_i, else_if) in if_stmt.else_if.iter().enumerate() {
                let jf_idx = self.emit_cond_jump_false(&else_if.condition, instructions);

                for s in &else_if.body {
                    self.compile_statement(s, instructions);
                }

                let je_idx = instructions.len();
                instructions.push(BytecodeOp::Jump(0));
                end_jumps.push(je_idx);

                let after_arm = instructions.len();
                Self::patch_cond_jump_target(instructions, jf_idx, after_arm);
            }

            // Else branch
            if let Some(else_branch) = &if_stmt.else_branch {
                for s in else_branch {
                    self.compile_statement(s, instructions);
                }
            }

            // Patch all end jumps
            let end_pos = instructions.len();
            for idx in end_jumps {
                instructions[idx] = BytecodeOp::Jump(end_pos);
            }
        } else {
            let end_of_then = instructions.len();
            Self::patch_cond_jump_target(instructions, jump_false_idx, end_of_then);
        }
    }

    /// Peephole: if `expr` is a postfix/prefix inc/dec of a plain identifier and
    /// If `expr` is `<identifier> <cmp> <int-literal>` (either side), returns
    /// `(name, const, op)` with `op` oriented so that truthiness means
    /// "identifier CMP const" — i.e. the condition is true when the
    /// comparison evaluates that way. Used by `compile_for` to fuse the loop
    /// condition into `JumpIfLocalCmpConstFalse`.
    fn match_local_cmp_const(expr: &Expression) -> Option<(String, i64, CmpOp)> {
        let bin = match expr {
            Expression::BinaryOp(b) => b,
            _ => return None,
        };
        let cmp = match bin.operator {
            BinaryOpType::Less => CmpOp::Lt,
            BinaryOpType::LessEqual => CmpOp::Lte,
            BinaryOpType::Greater => CmpOp::Gt,
            BinaryOpType::GreaterEqual => CmpOp::Gte,
            BinaryOpType::Equal => CmpOp::Eq,
            BinaryOpType::NotEqual => CmpOp::Neq,
            _ => return None,
        };
        let int_lit = |e: &Expression| -> Option<i64> {
            if let Expression::Literal(lit) = e {
                if let LiteralValue::Int(n) = &lit.value {
                    return Some(*n);
                }
            }
            None
        };
        let ident_name = |e: &Expression| -> Option<String> {
            if let Expression::Identifier(id) = e {
                Some(id.name.clone())
            } else {
                None
            }
        };
        if let (Some(name), Some(c)) = (ident_name(&bin.left), int_lit(&bin.right)) {
            Some((name, c, cmp))
        } else if let (Some(c), Some(name)) = (int_lit(&bin.left), ident_name(&bin.right)) {
            // `CONST <cmp> ident` — flip the op so the semantics stay right.
            let flipped = match cmp {
                CmpOp::Lt => CmpOp::Gt,
                CmpOp::Lte => CmpOp::Gte,
                CmpOp::Gt => CmpOp::Lt,
                CmpOp::Gte => CmpOp::Lte,
                CmpOp::Eq => CmpOp::Eq,
                CmpOp::Neq => CmpOp::Neq,
            };
            Some((name, c, flipped))
        } else {
            None
        }
    }

    /// Emit a condition followed by a "jump-if-false" exit. If the condition
    /// matches `<ident> <cmp> <int-const>`, emits a single fused
    /// JumpIfLocalCmpConstFalse. Otherwise compile_expression + JumpIfFalse.
    /// Returns the index of the jump op (so the caller can patch the target).
    fn emit_cond_jump_false(
        &mut self,
        condition: &Expression,
        instructions: &mut Vec<BytecodeOp>,
    ) -> usize {
        if let Some((name, c, cmp)) = Self::match_local_cmp_const(condition) {
            let idx = instructions.len();
            instructions.push(BytecodeOp::JumpIfLocalCmpConstFalse(name, c, cmp, 0));
            idx
        } else {
            self.compile_expression(condition, instructions);
            let idx = instructions.len();
            instructions.push(BytecodeOp::JumpIfFalse(0));
            idx
        }
    }

    /// Patch the jump target of either BytecodeOp::JumpIfFalse or the fused
    /// BytecodeOp::JumpIfLocalCmpConstFalse at `idx`.
    fn patch_cond_jump_target(instructions: &mut [BytecodeOp], idx: usize, target: usize) {
        match &mut instructions[idx] {
            BytecodeOp::JumpIfFalse(off) => *off = target,
            BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, off) => *off = target,
            _ => unreachable!("patch_cond_jump_target on unexpected op"),
        }
    }

    /// If `expr` advances a plain identifier by a constant integer step,
    /// returns `(name, step)`. Recognises all of:
    ///   - `i++` / `i--` / `++i` / `--i`       → step = ±1
    ///   - `i += K` / `i -= K` (int literal K)   → step = ±K
    ///   - `i = i + K` / `i = K + i` / `i = i - K` (int literal K)
    /// Used by compile_for to detect counted-loop shapes for ForLoopStep
    /// fusion; ForLoopStep encodes the step as an i64 so non-±1 strides
    /// like `i += 7` fuse too.
    fn match_inc_dec_identifier(expr: &Expression) -> Option<(String, i64)> {
        let int_lit = |e: &Expression| -> Option<i64> {
            if let Expression::Literal(lit) = e {
                if let LiteralValue::Int(n) = &lit.value {
                    return Some(*n);
                }
            }
            None
        };
        let ident_name = |e: &Expression| -> Option<String> {
            if let Expression::Identifier(id) = e {
                Some(id.name.clone())
            } else {
                None
            }
        };
        match expr {
            Expression::PostfixOp(postfix) => {
                if let Expression::Identifier(ident) = &*postfix.operand {
                    let step = match postfix.operator {
                        PostfixOpType::Increment => 1,
                        PostfixOpType::Decrement => -1,
                    };
                    return Some((ident.name.clone(), step));
                }
                None
            }
            Expression::UnaryOp(unary) => {
                if let Expression::Identifier(ident) = &*unary.operand {
                    let step = match unary.operator {
                        UnaryOpType::PrefixIncrement => 1,
                        UnaryOpType::PrefixDecrement => -1,
                        _ => return None,
                    };
                    return Some((ident.name.clone(), step));
                }
                None
            }
            // `i = i + K`, `i = K + i`, `i = i - K` — parser represents this
            // as a top-level BinaryOp with operator Assign, LHS the target
            // identifier and RHS the value expression.
            Expression::BinaryOp(outer) if matches!(outer.operator, BinaryOpType::Assign) => {
                let name = ident_name(&outer.left)?;
                let inner = match &*outer.right {
                    Expression::BinaryOp(b) => b,
                    _ => return None,
                };
                match inner.operator {
                    BinaryOpType::Add => {
                        if let (Some(l), Some(k)) =
                            (ident_name(&inner.left), int_lit(&inner.right))
                        {
                            if l == name {
                                return Some((name, k));
                            }
                        }
                        if let (Some(k), Some(r)) =
                            (int_lit(&inner.left), ident_name(&inner.right))
                        {
                            if r == name {
                                return Some((name, k));
                            }
                        }
                        None
                    }
                    BinaryOpType::Sub => {
                        if let (Some(l), Some(k)) =
                            (ident_name(&inner.left), int_lit(&inner.right))
                        {
                            if l == name {
                                return Some((name, -k));
                            }
                        }
                        None
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Statement-level counterpart to `match_inc_dec_identifier`. In a for
    /// loop the stride lives in the `increment` slot as an Expression, but
    /// in a while/do-while body it's a Statement — either an expression
    /// statement (`i++;`) or an Assignment (`i = i + 1;`, `i += K;`).
    /// Returns (name, step) on a match.
    fn match_stride_statement(stmt: &Statement) -> Option<(String, i64)> {
        let int_lit = |e: &Expression| -> Option<i64> {
            if let Expression::Literal(lit) = e {
                if let LiteralValue::Int(n) = &lit.value {
                    return Some(*n);
                }
            }
            None
        };
        let ident_name = |e: &Expression| -> Option<String> {
            if let Expression::Identifier(id) = e {
                Some(id.name.clone())
            } else {
                None
            }
        };
        match stmt {
            Statement::Expression(es) => Self::match_inc_dec_identifier(&es.expr),
            Statement::Assignment(a) => {
                let name = match &a.target {
                    AssignTarget::Variable(n) => n.clone(),
                    _ => return None,
                };
                match a.operator {
                    AssignOp::PlusEqual => int_lit(&a.value).map(|k| (name, k)),
                    AssignOp::MinusEqual => int_lit(&a.value).map(|k| (name, -k)),
                    AssignOp::Equal => {
                        // `i = i + K` / `i = K + i` / `i = i - K`
                        let inner = match &a.value {
                            Expression::BinaryOp(b) => b,
                            _ => return None,
                        };
                        match inner.operator {
                            BinaryOpType::Add => {
                                if let (Some(l), Some(k)) =
                                    (ident_name(&inner.left), int_lit(&inner.right))
                                {
                                    if l.eq_ignore_ascii_case(&name) {
                                        return Some((name, k));
                                    }
                                }
                                if let (Some(k), Some(r)) =
                                    (int_lit(&inner.left), ident_name(&inner.right))
                                {
                                    if r.eq_ignore_ascii_case(&name) {
                                        return Some((name, k));
                                    }
                                }
                                None
                            }
                            BinaryOpType::Sub => {
                                if let (Some(l), Some(k)) =
                                    (ident_name(&inner.left), int_lit(&inner.right))
                                {
                                    if l.eq_ignore_ascii_case(&name) {
                                        return Some((name, -k));
                                    }
                                }
                                None
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Returns true if the body contains a `continue` statement that would
    /// target THIS loop (i.e. not inside a nested loop or function body).
    /// Used to decide whether stride-hoisting is safe: hoisting the stride
    /// out changes what `continue` skips vs. runs, so we bail when a
    /// top-level continue is present.
    fn body_has_top_level_continue(body: &[Statement]) -> bool {
        for stmt in body {
            if Self::stmt_has_top_level_continue(stmt) {
                return true;
            }
        }
        false
    }

    fn stmt_has_top_level_continue(stmt: &Statement) -> bool {
        match stmt {
            Statement::Continue(_) => true,
            Statement::If(i) => {
                Self::body_has_top_level_continue(&i.then_branch)
                    || i.else_if
                        .iter()
                        .any(|ei| Self::body_has_top_level_continue(&ei.body))
                    || i.else_branch
                        .as_ref()
                        .map(|b| Self::body_has_top_level_continue(b))
                        .unwrap_or(false)
            }
            Statement::Switch(s) => {
                s.cases
                    .iter()
                    .any(|c| Self::body_has_top_level_continue(&c.body))
                    || s.default_case
                        .as_ref()
                        .map(|b| Self::body_has_top_level_continue(b))
                        .unwrap_or(false)
            }
            Statement::Try(t) => {
                Self::body_has_top_level_continue(&t.body)
                    || t.catches
                        .iter()
                        .any(|c| Self::body_has_top_level_continue(&c.body))
                    || t.finally_body
                        .as_ref()
                        .map(|b| Self::body_has_top_level_continue(b))
                        .unwrap_or(false)
            }
            Statement::Output(o) => Self::body_has_top_level_continue(&o.body),
            // Nested loops and function decls define their own continue target —
            // continues inside them don't target the outer loop.
            Statement::For(_)
            | Statement::ForIn(_)
            | Statement::While(_)
            | Statement::Do(_)
            | Statement::FunctionDecl(_)
            | Statement::ComponentDecl(_)
            | Statement::InterfaceDecl(_) => false,
            _ => false,
        }
    }

    /// its result is about to be discarded, emit a single `Increment` /
    /// `Decrement` op (pure side-effect, no stack push) and return true.
    /// Saves 5 ops → 1 op per iteration on tight `i++`-style loops, which is
    /// the dominant bytecode in `for (i=...;...;i++)` — the hottest loop shape
    /// in CFML.
    fn try_emit_inc_dec_statement(
        &mut self,
        expr: &Expression,
        instructions: &mut Vec<BytecodeOp>,
    ) -> bool {
        // Helper: emit `target op= 1` as a statement for any assignable expression.
        fn emit_memberaccess_step(
            this: &mut CfmlCompiler,
            access: &MemberAccess,
            delta: i64,
            instructions: &mut Vec<BytecodeOp>,
        ) {
            // obj.member = obj.member + delta  (statement form: no stack leftover)
            this.compile_expression(&access.object, instructions);
            instructions.push(BytecodeOp::GetProperty(access.member.clone()));
            instructions.push(BytecodeOp::Integer(delta));
            instructions.push(BytecodeOp::Add);
            // Store back via SetProperty + nested writeback
            this.compile_expression(&access.object, instructions);
            instructions.push(BytecodeOp::Swap);
            instructions.push(BytecodeOp::SetProperty(access.member.clone()));
            CfmlCompiler::emit_nested_writeback(&access.object, instructions);
        }

        match expr {
            Expression::PostfixOp(postfix) => {
                if let Expression::Identifier(ident) = &*postfix.operand {
                    match postfix.operator {
                        PostfixOpType::Increment => {
                            instructions.push(BytecodeOp::Increment(ident.name.clone()));
                            return true;
                        }
                        PostfixOpType::Decrement => {
                            instructions.push(BytecodeOp::Decrement(ident.name.clone()));
                            return true;
                        }
                    }
                }
                if let Expression::MemberAccess(access) = &*postfix.operand {
                    let delta = match postfix.operator {
                        PostfixOpType::Increment => 1,
                        PostfixOpType::Decrement => -1,
                    };
                    emit_memberaccess_step(self, access, delta, instructions);
                    return true;
                }
            }
            Expression::UnaryOp(unary) => {
                if let Expression::Identifier(ident) = &*unary.operand {
                    match unary.operator {
                        UnaryOpType::PrefixIncrement => {
                            instructions.push(BytecodeOp::Increment(ident.name.clone()));
                            return true;
                        }
                        UnaryOpType::PrefixDecrement => {
                            instructions.push(BytecodeOp::Decrement(ident.name.clone()));
                            return true;
                        }
                        _ => {}
                    }
                }
                if let Expression::MemberAccess(access) = &*unary.operand {
                    let delta = match unary.operator {
                        UnaryOpType::PrefixIncrement => 1,
                        UnaryOpType::PrefixDecrement => -1,
                        _ => return false,
                    };
                    emit_memberaccess_step(self, access, delta, instructions);
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn compile_for(&mut self, for_stmt: &For, instructions: &mut Vec<BytecodeOp>) {
        // Init
        if let Some(init) = &for_stmt.init {
            self.compile_statement(init, instructions);
        }

        // Counted-loop fusion: if both
        //   - condition is  <ident> <cmp> <int-const>
        //   - increment is  i++ / i-- / ++i / --i on the same identifier
        // then emit the specialized do-while-ish shape with ForLoopStep at
        // the bottom, dropping per-iter overhead from 3 ops (Increment,
        // JumpIfLocalCmpConstFalse, Jump) to 1 op (ForLoopStep).
        if let Some(condition) = &for_stmt.condition {
            if let (Some((cond_name, c, cmp)), Some(increment)) =
                (Self::match_local_cmp_const(condition), for_stmt.increment.as_deref())
            {
                if let Some((inc_name, step)) = Self::match_inc_dec_identifier(increment) {
                    if cond_name == inc_name {
                        self.compile_for_counted(
                            &cond_name, c, cmp, step, &for_stmt.body, instructions,
                        );
                        return;
                    }
                }
            }
        }

        // Fallback: the generic peephole'd shape.
        let loop_start = instructions.len();

        if let Some(condition) = &for_stmt.condition {
            let jump_false_idx = if let Some((name, c, cmp)) =
                Self::match_local_cmp_const(condition)
            {
                let idx = instructions.len();
                instructions.push(BytecodeOp::JumpIfLocalCmpConstFalse(name, c, cmp, 0));
                idx
            } else {
                self.compile_expression(condition, instructions);
                let idx = instructions.len();
                instructions.push(BytecodeOp::JumpIfFalse(0));
                idx
            };

            self.loop_stack.push((Vec::new(), Vec::new()));

            for s in &for_stmt.body {
                self.compile_statement(s, instructions);
            }

            let continue_target = instructions.len();

            if let Some(increment) = &for_stmt.increment {
                if !self.try_emit_inc_dec_statement(increment, instructions) {
                    self.compile_expression(increment, instructions);
                    instructions.push(BytecodeOp::Pop);
                }
            }

            instructions.push(BytecodeOp::Jump(loop_start));

            let loop_end = instructions.len();
            match &mut instructions[jump_false_idx] {
                BytecodeOp::JumpIfFalse(off) => *off = loop_end,
                BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, off) => *off = loop_end,
                _ => unreachable!("compile_for exit jump slot has unexpected op"),
            }

            let (break_indices, continue_indices) = self.loop_stack.pop().unwrap();
            for idx in break_indices {
                instructions[idx] = BytecodeOp::Jump(loop_end);
            }
            for idx in continue_indices {
                instructions[idx] = BytecodeOp::Jump(continue_target);
            }
        }
    }

    /// Emit the counted-for-loop shape using ForLoopStep.
    /// The variable `name` must match between condition and increment.
    fn compile_for_counted(
        &mut self,
        name: &str,
        limit: i64,
        cmp: CmpOp,
        step: i64,
        body: &[Statement],
        instructions: &mut Vec<BytecodeOp>,
    ) {
        // Initial check: if the condition is already false at entry, skip
        // the loop entirely. Emits one op; the target is patched to loop_end.
        let entry_check_idx = instructions.len();
        instructions.push(BytecodeOp::JumpIfLocalCmpConstFalse(
            name.to_string(), limit, cmp, 0,
        ));

        let body_start = instructions.len();

        self.loop_stack.push((Vec::new(), Vec::new()));

        for s in body {
            self.compile_statement(s, instructions);
        }

        // continue target = the step — continue runs the step, then re-tests.
        let continue_target = instructions.len();
        instructions.push(BytecodeOp::ForLoopStep(
            name.to_string(), limit, cmp, step, body_start,
        ));

        let loop_end = instructions.len();

        // Patch the entry-check to exit to loop_end if condition initially false.
        if let BytecodeOp::JumpIfLocalCmpConstFalse(_, _, _, off) =
            &mut instructions[entry_check_idx]
        {
            *off = loop_end;
        }

        let (break_indices, continue_indices) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(continue_target);
        }
    }

    fn compile_for_in(&mut self, for_in: &ForIn, instructions: &mut Vec<BytecodeOp>) {
        // Compile iterable
        self.compile_expression(&for_in.iterable, instructions);

        // GetKeys: if struct, convert to array of keys; arrays pass through unchanged
        instructions.push(BytecodeOp::GetKeys);

        // Unique per-loop temp names (so nested for-in don't collide).
        let iter_var = format!("__iter_{}", instructions.len());
        let idx_var = format!("__idx_{}", instructions.len());
        let limit_var = format!("__limit_{}", instructions.len());
        // Declare as function-locals so StoreLocal writes to locals (not __variables
        // in a CFC method context) — otherwise the loop counter never increments.
        instructions.push(BytecodeOp::DeclareLocal(iter_var.clone()));
        instructions.push(BytecodeOp::DeclareLocal(idx_var.clone()));
        instructions.push(BytecodeOp::DeclareLocal(limit_var.clone()));
        instructions.push(BytecodeOp::StoreLocal(iter_var.clone()));

        // Hoist len(iterable) out of the loop. The old codegen looked up the
        // `len` builtin and invoked it every iteration — a HashMap probe plus
        // full function-call trampoline per element. Compute once, reuse.
        instructions.push(BytecodeOp::LoadGlobal("len".to_string()));
        instructions.push(BytecodeOp::LoadLocal(iter_var.clone()));
        instructions.push(BytecodeOp::Call(1));
        instructions.push(BytecodeOp::StoreLocal(limit_var.clone()));

        // CFML arrays are 1-based, so start index at 1.
        instructions.push(BytecodeOp::Integer(1));
        instructions.push(BytecodeOp::StoreLocal(idx_var.clone()));

        let loop_start = instructions.len();

        // Condition: idx <= limit  (both locals; no builtin call per iter).
        instructions.push(BytecodeOp::LoadLocal(idx_var.clone()));
        instructions.push(BytecodeOp::LoadLocal(limit_var.clone()));
        instructions.push(BytecodeOp::Lte);

        let jump_false_idx = instructions.len();
        instructions.push(BytecodeOp::JumpIfFalse(0));

        // Set loop variable = iterable[idx]
        instructions.push(BytecodeOp::LoadLocal(iter_var.clone()));
        instructions.push(BytecodeOp::LoadLocal(idx_var.clone()));
        instructions.push(BytecodeOp::GetIndex);
        // Strip a leading `local.` prefix from the loop variable so it stores
        // as a simple local rather than a literal key containing a dot. A
        // subsequent `local.X` read resolves to that local via the normal
        // locals lookup.
        let loop_var_name = if let Some(rest) = for_in.variable.strip_prefix("local.") {
            rest.to_string()
        } else {
            for_in.variable.clone()
        };
        if loop_var_name.contains('.') {
            // Member-path loop variable (e.g. `ctx.item`, `this.wheels.folder`).
            // Lucee/ACF/BoxLang assign the iterated value through the path each
            // iteration. Emit a struct write-back chain: load the deepest
            // parent, set the leaf property, then propagate the modified
            // struct back up to the root local.
            let segments: Vec<String> =
                loop_var_name.split('.').map(|s| s.to_string()).collect();
            let root = segments[0].clone();
            let leaf = segments[segments.len() - 1].clone();
            let intermediate = &segments[1..segments.len() - 1];
            // Stack on entry: [element_value]
            // Load deepest parent: root[.intermediate[0]...intermediate[n]]
            instructions.push(BytecodeOp::LoadLocal(root.clone()));
            for seg in intermediate {
                instructions.push(BytecodeOp::GetProperty(seg.clone()));
            }
            // Stack: [element_value, deepest_parent]
            instructions.push(BytecodeOp::Swap);
            instructions.push(BytecodeOp::SetProperty(leaf));
            // Stack: [modified_deepest_parent]
            // Unwind: for each intermediate level (deepest -> shallowest),
            // reload its parent and SetProperty back in.
            for i in (0..intermediate.len()).rev() {
                instructions.push(BytecodeOp::LoadLocal(root.clone()));
                for seg in &intermediate[..i] {
                    instructions.push(BytecodeOp::GetProperty(seg.clone()));
                }
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(intermediate[i].clone()));
            }
            // Stack: [modified_root]
            instructions.push(BytecodeOp::StoreLocal(root));
        } else {
            instructions.push(BytecodeOp::DeclareLocal(loop_var_name.clone()));
            instructions.push(BytecodeOp::StoreLocal(loop_var_name));
        }

        self.loop_stack.push((Vec::new(), Vec::new()));

        for s in &for_in.body {
            self.compile_statement(s, instructions);
        }

        let continue_target = instructions.len();

        // idx++  (single Increment op, not Load+Int+Add+Store).
        instructions.push(BytecodeOp::Increment(idx_var.clone()));

        instructions.push(BytecodeOp::Jump(loop_start));

        let loop_end = instructions.len();
        instructions[jump_false_idx] = BytecodeOp::JumpIfFalse(loop_end);

        let (break_indices, continue_indices) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(continue_target);
        }
    }

    fn compile_while(&mut self, while_stmt: &While, instructions: &mut Vec<BytecodeOp>) {
        // Counted-loop fusion: if the condition is `<ident> <cmp> <int-const>`
        // and the last body statement advances the same identifier by a
        // constant step (i++, i+=K, i = i+K, etc.), hoist the stride out and
        // emit the same ForLoopStep-based shape used by compile_for_counted.
        // Skip when a top-level `continue` is present — the hoist would
        // change whether `continue` runs the stride.
        if let Some((cond_name, c, cmp)) = Self::match_local_cmp_const(&while_stmt.condition) {
            if let Some(last) = while_stmt.body.last() {
                if let Some((stride_name, step)) = Self::match_stride_statement(last) {
                    if cond_name.eq_ignore_ascii_case(&stride_name)
                        && !Self::body_has_top_level_continue(&while_stmt.body)
                    {
                        let body_without_stride =
                            &while_stmt.body[..while_stmt.body.len() - 1];
                        self.compile_for_counted(
                            &cond_name, c, cmp, step, body_without_stride, instructions,
                        );
                        return;
                    }
                }
            }
        }

        let loop_start = instructions.len();

        let jump_false_idx = self.emit_cond_jump_false(&while_stmt.condition, instructions);

        self.loop_stack.push((Vec::new(), Vec::new()));

        for s in &while_stmt.body {
            self.compile_statement(s, instructions);
        }

        instructions.push(BytecodeOp::Jump(loop_start));

        let loop_end = instructions.len();
        Self::patch_cond_jump_target(instructions, jump_false_idx, loop_end);

        let (break_indices, continue_indices) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(loop_start);
        }
    }

    fn compile_do(&mut self, do_stmt: &Do, instructions: &mut Vec<BytecodeOp>) {
        // Counted-do-while fusion: same shape as compile_while but no entry
        // check — do-while always runs the body at least once.
        if let Some((cond_name, c, cmp)) = Self::match_local_cmp_const(&do_stmt.condition) {
            if let Some(last) = do_stmt.body.last() {
                if let Some((stride_name, step)) = Self::match_stride_statement(last) {
                    if cond_name.eq_ignore_ascii_case(&stride_name)
                        && !Self::body_has_top_level_continue(&do_stmt.body)
                    {
                        let body_without_stride = &do_stmt.body[..do_stmt.body.len() - 1];
                        self.compile_do_counted(
                            &cond_name, c, cmp, step, body_without_stride, instructions,
                        );
                        return;
                    }
                }
            }
        }

        let loop_start = instructions.len();

        self.loop_stack.push((Vec::new(), Vec::new()));

        for s in &do_stmt.body {
            self.compile_statement(s, instructions);
        }

        let continue_target = instructions.len();

        self.compile_expression(&do_stmt.condition, instructions);
        instructions.push(BytecodeOp::JumpIfTrue(loop_start));

        let loop_end = instructions.len();

        let (break_indices, continue_indices) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(continue_target);
        }
    }

    /// Counted-do-while fused shape: no entry check (body always runs once),
    /// stride folded into the bottom ForLoopStep.
    fn compile_do_counted(
        &mut self,
        name: &str,
        limit: i64,
        cmp: CmpOp,
        step: i64,
        body: &[Statement],
        instructions: &mut Vec<BytecodeOp>,
    ) {
        let body_start = instructions.len();

        self.loop_stack.push((Vec::new(), Vec::new()));

        for s in body {
            self.compile_statement(s, instructions);
        }

        let continue_target = instructions.len();
        instructions.push(BytecodeOp::ForLoopStep(
            name.to_string(), limit, cmp, step, body_start,
        ));

        let loop_end = instructions.len();

        let (break_indices, continue_indices) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(loop_end);
        }
        for idx in continue_indices {
            instructions[idx] = BytecodeOp::Jump(continue_target);
        }
    }

    fn compile_switch(&mut self, switch_stmt: &Switch, instructions: &mut Vec<BytecodeOp>) {
        // Evaluate switch expression and store
        self.compile_expression(&switch_stmt.expression, instructions);
        let switch_var = format!("__switch_{}", instructions.len());
        instructions.push(BytecodeOp::StoreLocal(switch_var.clone()));

        self.loop_stack.push((Vec::new(), Vec::new())); // break support

        let mut end_jumps = Vec::new();
        let mut next_case_jump: Option<usize> = None;

        for case in &switch_stmt.cases {
            // Patch previous case's fall-through check
            if let Some(prev_jump) = next_case_jump {
                instructions[prev_jump] = BytecodeOp::JumpIfFalse(instructions.len());
            }

            // Compare switch value to case value(s)
            // For multiple values, OR them together
            for (i, val) in case.values.iter().enumerate() {
                instructions.push(BytecodeOp::LoadLocal(switch_var.clone()));
                self.compile_expression(val, instructions);
                instructions.push(BytecodeOp::Eq);

                if i > 0 {
                    instructions.push(BytecodeOp::Or);
                }
            }

            next_case_jump = Some(instructions.len());
            instructions.push(BytecodeOp::JumpIfFalse(0));

            for s in &case.body {
                self.compile_statement(s, instructions);
            }

            // Jump to end after case body (unless there's a break which does this too)
            let je_idx = instructions.len();
            instructions.push(BytecodeOp::Jump(0));
            end_jumps.push(je_idx);
        }

        // Patch last case jump
        if let Some(prev_jump) = next_case_jump {
            instructions[prev_jump] = BytecodeOp::JumpIfFalse(instructions.len());
        }

        // Default case
        if let Some(default) = &switch_stmt.default_case {
            for s in default {
                self.compile_statement(s, instructions);
            }
        }

        let end_pos = instructions.len();
        for idx in end_jumps {
            instructions[idx] = BytecodeOp::Jump(end_pos);
        }

        // Patch break statements
        let (break_indices, _) = self.loop_stack.pop().unwrap();
        for idx in break_indices {
            instructions[idx] = BytecodeOp::Jump(end_pos);
        }
    }

    fn compile_try(&mut self, try_stmt: &Try, instructions: &mut Vec<BytecodeOp>) {
        // TryStart points to catch handler
        let try_start_idx = instructions.len();
        instructions.push(BytecodeOp::TryStart(0)); // placeholder

        // Try body
        for s in &try_stmt.body {
            self.compile_statement(s, instructions);
        }
        instructions.push(BytecodeOp::TryEnd);

        // Jump over catch blocks
        let jump_over_catch = instructions.len();
        instructions.push(BytecodeOp::Jump(0));

        // Catch handler
        let catch_start = instructions.len();
        instructions[try_start_idx] = BytecodeOp::TryStart(catch_start);

        // Set current_finally so that rethrow can emit finally body first
        let prev_finally = self.current_finally.take();
        if let Some(ref finally_body) = try_stmt.finally_body {
            self.current_finally = Some(finally_body.clone());
        }

        for catch in &try_stmt.catches {
            // The error value will be on the stack
            instructions.push(BytecodeOp::StoreLocal(catch.var_name.clone()));

            for s in &catch.body {
                self.compile_statement(s, instructions);
            }
        }

        // Restore previous finally context
        self.current_finally = prev_finally;

        let end_pos = instructions.len();
        instructions[jump_over_catch] = BytecodeOp::Jump(end_pos);

        // Finally
        if let Some(finally_body) = &try_stmt.finally_body {
            for s in finally_body {
                self.compile_statement(s, instructions);
            }
        }
    }

    fn compile_function_decl(&mut self, func: &Function, instructions: &mut Vec<BytecodeOp>) {
        // Compile the function body into a separate BytecodeFunction
        let mut func_instructions = Vec::new();

        self.function_depth += 1;

        // Save/restore the surrounding function's declared localMode so nested
        // closures inherit *this* function's mode rather than something further
        // up the stack.
        let declared_mode = metadata_declared_local_mode(&func.metadata);
        let prev_fn_local_mode = self.current_fn_local_mode;
        self.current_fn_local_mode = declared_mode.or(prev_fn_local_mode);

        // Emit default parameter value preamble:
        // For each param with a default, if the arg is null, assign the default
        // and also update the arguments scope
        for param in &func.params {
            if let Some(ref default_expr) = param.default {
                func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                func_instructions.push(BytecodeOp::IsNull);
                let jump_idx = func_instructions.len();
                func_instructions.push(BytecodeOp::JumpIfFalse(0)); // placeholder
                // Set the local variable
                self.compile_expression(default_expr, &mut func_instructions);
                func_instructions.push(BytecodeOp::StoreLocal(param.name.clone()));
                // Also update the arguments scope
                func_instructions.push(BytecodeOp::LoadLocal("arguments".to_string()));
                func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                func_instructions.push(BytecodeOp::SetProperty(param.name.clone()));
                func_instructions.push(BytecodeOp::StoreLocal("arguments".to_string()));
                func_instructions[jump_idx] = BytecodeOp::JumpIfFalse(func_instructions.len());
            }
        }

        for s in &func.body {
            self.compile_statement(s, &mut func_instructions);
        }

        // Ensure function returns null if no explicit return
        func_instructions.push(BytecodeOp::Null);
        func_instructions.push(BytecodeOp::Return);

        self.function_depth -= 1;
        self.current_fn_local_mode = prev_fn_local_mode;

        let bc_func = BytecodeFunction {
            name: func.name.clone(),
            params: func.params.iter().map(|p| p.name.clone()).collect(),
            required_params: func.params.iter().map(|p| p.required).collect(),
            instructions: func_instructions,
            source_file: None,
            declared_local_mode: declared_mode,
        };

        let func_idx = self.program.functions.len();
        self.program.functions.push(Arc::new(bc_func));

        // Define the function in current scope
        instructions.push(BytecodeOp::DefineFunction(func_idx));
        instructions.push(BytecodeOp::StoreLocal(func.name.clone()));
    }

    fn compile_interface(&mut self, interface: &Interface, instructions: &mut Vec<BytecodeOp>) {
        let mut prop_count = 0;

        // __is_interface marker
        instructions.push(BytecodeOp::String("__is_interface".to_string()));
        instructions.push(BytecodeOp::True);
        prop_count += 1;

        // __name
        instructions.push(BytecodeOp::String("__name".to_string()));
        instructions.push(BytecodeOp::String(interface.name.clone()));
        prop_count += 1;

        // __extends array (interfaces can extend multiple parents)
        if !interface.extends.is_empty() {
            instructions.push(BytecodeOp::String("__extends".to_string()));
            for parent in &interface.extends {
                instructions.push(BytecodeOp::String(parent.clone()));
            }
            instructions.push(BytecodeOp::BuildArray(interface.extends.len()));
            prop_count += 1;
        }

        // __methods struct: { method_name_lc: { name, params, returnType, access } }
        if !interface.functions.is_empty() {
            instructions.push(BytecodeOp::String("__methods".to_string()));
            for func in &interface.functions {
                let method_key = func.name.to_lowercase();
                instructions.push(BytecodeOp::String(method_key));

                let mut method_prop_count = 0;

                // name
                instructions.push(BytecodeOp::String("name".to_string()));
                instructions.push(BytecodeOp::String(func.name.clone()));
                method_prop_count += 1;

                // returnType
                if let Some(ref rt) = func.return_type {
                    instructions.push(BytecodeOp::String("returnType".to_string()));
                    instructions.push(BytecodeOp::String(rt.clone()));
                    method_prop_count += 1;
                }

                // access
                let access_str = match func.access {
                    AccessModifier::Public => "public",
                    AccessModifier::Private => "private",
                    AccessModifier::Package => "package",
                    AccessModifier::Remote => "remote",
                };
                instructions.push(BytecodeOp::String("access".to_string()));
                instructions.push(BytecodeOp::String(access_str.to_string()));
                method_prop_count += 1;

                // params array
                if !func.params.is_empty() {
                    instructions.push(BytecodeOp::String("params".to_string()));
                    for param in &func.params {
                        instructions.push(BytecodeOp::String(param.name.clone()));
                    }
                    instructions.push(BytecodeOp::BuildArray(func.params.len()));
                    method_prop_count += 1;
                }

                instructions.push(BytecodeOp::BuildStruct(method_prop_count));
            }
            instructions.push(BytecodeOp::BuildStruct(interface.functions.len()));
            prop_count += 1;
        }

        // __metadata
        if !interface.metadata.is_empty() {
            instructions.push(BytecodeOp::String("__metadata".to_string()));
            for (k, v) in &interface.metadata {
                instructions.push(BytecodeOp::String(k.clone()));
                instructions.push(BytecodeOp::String(v.clone()));
            }
            instructions.push(BytecodeOp::BuildStruct(interface.metadata.len()));
            prop_count += 1;
        }

        // Build the interface struct
        instructions.push(BytecodeOp::BuildStruct(prop_count));

        // Store in local and global scope (same as component)
        instructions.push(BytecodeOp::StoreLocal(interface.name.clone()));
        instructions.push(BytecodeOp::LoadLocal(interface.name.clone()));
        instructions.push(BytecodeOp::StoreGlobal(interface.name.clone()));
    }

    fn compile_component(&mut self, component: &Component, instructions: &mut Vec<BytecodeOp>) {
        // Build the component as a struct containing:
        // 1. Metadata keys (__name, __extends, __implements, __metadata)
        // 2. __variables scope with property defaults
        // 3. Compiled methods as function references
        let mut prop_count = 0;

        // Add __name metadata
        instructions.push(BytecodeOp::String("__name".to_string()));
        instructions.push(BytecodeOp::String(component.name.clone()));
        prop_count += 1;

        // Add __extends if component extends another
        if let Some(ref ext) = component.extends {
            instructions.push(BytecodeOp::String("__extends".to_string()));
            instructions.push(BytecodeOp::String(ext.clone()));
            prop_count += 1;
        }

        // Add __implements if component implements interfaces
        if !component.implements.is_empty() {
            instructions.push(BytecodeOp::String("__implements".to_string()));
            for iface_name in &component.implements {
                instructions.push(BytecodeOp::String(iface_name.clone()));
            }
            instructions.push(BytecodeOp::BuildArray(component.implements.len()));
            prop_count += 1;
        }

        // Add __metadata sub-struct if component has metadata attributes
        if !component.metadata.is_empty() {
            instructions.push(BytecodeOp::String("__metadata".to_string()));
            for (k, v) in &component.metadata {
                instructions.push(BytecodeOp::String(k.clone()));
                instructions.push(BytecodeOp::String(v.clone()));
            }
            instructions.push(BytecodeOp::BuildStruct(component.metadata.len()));
            prop_count += 1;
        }

        // Add __variables scope for component properties (needed for accessors)
        // Include property defaults here
        if component.accessors || !component.properties.is_empty() {
            instructions.push(BytecodeOp::String("__variables".to_string()));
            // Build __variables struct with property defaults
            let mut vars_count = 0;
            for prop in &component.properties {
                instructions.push(BytecodeOp::String(prop.name.clone()));
                if let Some(default) = &prop.default {
                    self.compile_expression(default, instructions);
                } else {
                    instructions.push(BytecodeOp::Null);
                }
                vars_count += 1;
            }
            instructions.push(BytecodeOp::BuildStruct(vars_count));
            prop_count += 1;
        }

        // Build the base struct
        instructions.push(BytecodeOp::BuildStruct(prop_count));

        // Store as a component template in local scope first
        instructions.push(BytecodeOp::StoreLocal(component.name.clone()));

        // Generate accessor methods if accessors="true" (BEFORE storing globally)
        if component.accessors {
            for prop in &component.properties {
                // Generate getter: getPropertyName()
                let getter_name = format!("get{}", capitalize_first(&prop.name));
                let getter_func = BytecodeFunction {
                    name: getter_name.clone(),
                    params: Vec::new(),
                    required_params: Vec::new(),
                    instructions: vec![
                        BytecodeOp::LoadLocal("this".to_string()),
                        BytecodeOp::GetProperty(prop.name.clone()),
                        BytecodeOp::Return,
                    ],
                    source_file: None,
                    declared_local_mode: None,
                };
                let getter_idx = self.program.functions.len();
                self.program.functions.push(Arc::new(getter_func));
                instructions.push(BytecodeOp::DefineFunction(getter_idx));
                // Stack: [getter_func]

                // Add getter to component: component[getter_name] = getter_func
                // Stack: [getter_func]
                // Load component: [getter_func, component]
                // Swap: [component, getter_func]
                // SetProperty(getter_name): sets component.getter_name = getter_func, stack is [component]
                // StoreLocal: []
                instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(getter_name.clone()));
                instructions.push(BytecodeOp::StoreLocal(component.name.clone()));

                // Generate setter: setPropertyName(value)
                // Set the property directly on this struct and __variables
                let setter_name = format!("set{}", capitalize_first(&prop.name));
                let setter_func = BytecodeFunction {
                    name: setter_name.clone(),
                    params: vec![prop.name.clone()],
                    required_params: vec![true],
                    instructions: vec![
                        // Set on this: this.name = value; store modified this back
                        BytecodeOp::LoadLocal("this".to_string()),
                        BytecodeOp::LoadLocal(prop.name.clone()),
                        BytecodeOp::SetProperty(prop.name.clone()),
                        BytecodeOp::StoreLocal("this".to_string()),
                        // Set on __variables: this.__variables.name = value
                        BytecodeOp::LoadLocal("this".to_string()),
                        BytecodeOp::GetProperty("__variables".to_string()),
                        BytecodeOp::LoadLocal(prop.name.clone()),
                        BytecodeOp::SetProperty(prop.name.clone()),
                        BytecodeOp::StoreLocal("__variables".to_string()),
                        // Return this
                        BytecodeOp::LoadLocal("this".to_string()),
                        BytecodeOp::Return,
                    ],
                    source_file: None,
                    declared_local_mode: None,
                };
                let setter_idx = self.program.functions.len();
                self.program.functions.push(Arc::new(setter_func));
                instructions.push(BytecodeOp::DefineFunction(setter_idx));
                // Stack: [setter_func]

                // Add setter to component (same pattern)
                instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(setter_name.clone()));
                instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
            }
        }

        // Now store as a component template in global scope (with accessors included)
        instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
        instructions.push(BytecodeOp::StoreGlobal(component.name.clone()));

        // Compile component methods and add them to the component struct
        for func in &component.functions {
            self.compile_function_decl(func, instructions);
            // SetProperty needs: stack = [object, value]
            // Load the component struct, then load the function ref
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::LoadLocal(func.name.clone()));
            instructions.push(BytecodeOp::SetProperty(func.name.clone()));
            instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
        }

        // Emit per-function metadata as __funcmeta_<name> keys
        for func in &component.functions {
            if !func.metadata.is_empty() {
                let meta_key = format!("__funcmeta_{}", func.name);
                for (k, v) in &func.metadata {
                    instructions.push(BytecodeOp::String(k.clone()));
                    instructions.push(BytecodeOp::String(v.clone()));
                }
                instructions.push(BytecodeOp::BuildStruct(func.metadata.len()));
                instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
                instructions.push(BytecodeOp::Swap);
                instructions.push(BytecodeOp::SetProperty(meta_key));
                instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
            }
        }

        // Emit __properties array listing property metadata structs
        if !component.properties.is_empty() {
            let prop_count = component.properties.len();
            for prop in &component.properties {
                // Each property is a struct with name, type, required, and any custom attributes
                let mut attr_count = 1; // always have "name"
                instructions.push(BytecodeOp::String("name".to_string()));
                instructions.push(BytecodeOp::String(prop.name.clone()));
                if let Some(ref pt) = prop.prop_type {
                    instructions.push(BytecodeOp::String("type".to_string()));
                    instructions.push(BytecodeOp::String(pt.clone()));
                    attr_count += 1;
                }
                if prop.required {
                    instructions.push(BytecodeOp::String("required".to_string()));
                    instructions.push(BytecodeOp::True);
                    attr_count += 1;
                }
                // Custom attributes (inject, hint, etc.)
                for (key, val) in &prop.attributes {
                    instructions.push(BytecodeOp::String(key.clone()));
                    instructions.push(BytecodeOp::String(val.clone()));
                    attr_count += 1;
                }
                instructions.push(BytecodeOp::BuildStruct(attr_count));
            }
            instructions.push(BytecodeOp::BuildArray(prop_count));
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::Swap);
            instructions.push(BytecodeOp::SetProperty("__properties".to_string()));
            instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
        }

        // Update global copy after methods and metadata are added
        if !component.functions.is_empty() || !component.metadata.is_empty() || !component.properties.is_empty() {
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::StoreGlobal(component.name.clone()));
        }

        // Compile component body statements (e.g., this.name = "xxx", this.mappings = {...})
        // These execute as init code that modifies the component struct via `this`
        if !component.body.is_empty() {
            // Bind `this` to the component struct so `this.xxx = val` works
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::StoreLocal("this".to_string()));

            for stmt in &component.body {
                self.compile_statement(stmt, instructions);
            }

            // Copy modified `this` back to component name and global
            instructions.push(BytecodeOp::LoadLocal("this".to_string()));
            instructions.push(BytecodeOp::StoreLocal(component.name.clone()));
            instructions.push(BytecodeOp::LoadLocal(component.name.clone()));
            instructions.push(BytecodeOp::StoreGlobal(component.name.clone()));
        }
    }

    fn compile_expression(&mut self, expr: &Expression, instructions: &mut Vec<BytecodeOp>) {
        match expr {
            Expression::Literal(lit) => match &lit.value {
                LiteralValue::Null => instructions.push(BytecodeOp::Null),
                LiteralValue::Bool(true) => instructions.push(BytecodeOp::True),
                LiteralValue::Bool(false) => instructions.push(BytecodeOp::False),
                LiteralValue::Int(i) => instructions.push(BytecodeOp::Integer(*i)),
                LiteralValue::Double(d) => instructions.push(BytecodeOp::Double(*d)),
                LiteralValue::String(s) => instructions.push(BytecodeOp::String(s.clone())),
            },
            Expression::Identifier(id) => {
                instructions.push(BytecodeOp::LoadLocal(id.name.clone()));
            }
            Expression::BinaryOp(binop) => {
                if binop.operator == BinaryOpType::Assign {
                    self.compile_expression(&binop.right, instructions);

                    match &*binop.left {
                        Expression::Identifier(ident) => {
                            instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
                        }
                        Expression::MemberAccess(access) => {
                            // Stack has [value]. SetProperty needs [obj, value].
                            self.compile_expression(&access.object, instructions);
                            instructions.push(BytecodeOp::Swap);
                            instructions.push(BytecodeOp::SetProperty(access.member.clone()));
                            // Write back through nested chain
                            Self::emit_nested_writeback(&access.object, instructions);
                        }
                        Expression::ArrayAccess(access) => {
                            self.compile_expression(&access.array, instructions);
                            self.compile_expression(&access.index, instructions);
                            instructions.push(BytecodeOp::SetIndex);
                            // SetIndex leaves modified collection on stack; write it back
                            Self::emit_nested_writeback(&access.array, instructions);
                        }
                        _ => {}
                    }
                    return;
                }

                self.compile_expression(&binop.left, instructions);
                self.compile_expression(&binop.right, instructions);

                let op = match binop.operator {
                    BinaryOpType::Add => BytecodeOp::Add,
                    BinaryOpType::Sub => BytecodeOp::Sub,
                    BinaryOpType::Mul => BytecodeOp::Mul,
                    BinaryOpType::Div => BytecodeOp::Div,
                    BinaryOpType::Mod => BytecodeOp::Mod,
                    BinaryOpType::Pow => BytecodeOp::Pow,
                    BinaryOpType::IntDiv => BytecodeOp::IntDiv,
                    BinaryOpType::Concat => BytecodeOp::Concat,
                    BinaryOpType::Equal => BytecodeOp::Eq,
                    BinaryOpType::NotEqual => BytecodeOp::Neq,
                    BinaryOpType::Less => BytecodeOp::Lt,
                    BinaryOpType::LessEqual => BytecodeOp::Lte,
                    BinaryOpType::Greater => BytecodeOp::Gt,
                    BinaryOpType::GreaterEqual => BytecodeOp::Gte,
                    BinaryOpType::And => BytecodeOp::And,
                    BinaryOpType::Or => BytecodeOp::Or,
                    BinaryOpType::Xor => BytecodeOp::Xor,
                    BinaryOpType::Contains => BytecodeOp::Contains,
                    BinaryOpType::DoesNotContain => BytecodeOp::DoesNotContain,
                    BinaryOpType::Eqv => BytecodeOp::Eqv,
                    BinaryOpType::Imp => BytecodeOp::Imp,
                    BinaryOpType::Assign => BytecodeOp::Null, // Should not reach here
                };
                instructions.push(op);
            }
            Expression::UnaryOp(unary) => {
                match unary.operator {
                    UnaryOpType::PrefixIncrement | UnaryOpType::PrefixDecrement => {
                        // ++i / --i: increment/decrement and leave NEW value on stack
                        if let Expression::Identifier(ident) = &*unary.operand {
                            instructions.push(BytecodeOp::LoadLocal(ident.name.clone()));
                            instructions.push(BytecodeOp::Integer(1));
                            if matches!(unary.operator, UnaryOpType::PrefixIncrement) {
                                instructions.push(BytecodeOp::Add);
                            } else {
                                instructions.push(BytecodeOp::Sub);
                            }
                            instructions.push(BytecodeOp::Dup);
                            instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
                        } else {
                            // Fallback: evaluate operand, add/subtract 1
                            self.compile_expression(&unary.operand, instructions);
                            instructions.push(BytecodeOp::Integer(1));
                            if matches!(unary.operator, UnaryOpType::PrefixIncrement) {
                                instructions.push(BytecodeOp::Add);
                            } else {
                                instructions.push(BytecodeOp::Sub);
                            }
                        }
                    }
                    _ => {
                        self.compile_expression(&unary.operand, instructions);
                        let op = match unary.operator {
                            UnaryOpType::Minus => BytecodeOp::Negate,
                            UnaryOpType::Not => BytecodeOp::Not,
                            UnaryOpType::BitNot => BytecodeOp::Not,
                            _ => unreachable!(),
                        };
                        instructions.push(op);
                    }
                }
            }
            Expression::PostfixOp(postfix) => {
                if let Expression::Identifier(ident) = &*postfix.operand {
                    match postfix.operator {
                        PostfixOpType::Increment => {
                            instructions.push(BytecodeOp::LoadLocal(ident.name.clone()));
                            instructions.push(BytecodeOp::Dup);
                            instructions.push(BytecodeOp::Integer(1));
                            instructions.push(BytecodeOp::Add);
                            instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
                            // The original value stays on the stack
                        }
                        PostfixOpType::Decrement => {
                            instructions.push(BytecodeOp::LoadLocal(ident.name.clone()));
                            instructions.push(BytecodeOp::Dup);
                            instructions.push(BytecodeOp::Integer(1));
                            instructions.push(BytecodeOp::Sub);
                            instructions.push(BytecodeOp::StoreLocal(ident.name.clone()));
                        }
                    }
                }
            }
            Expression::MemberAccess(access) => {
                // Phase H peephole: at page scope, `variables.foo` clones the entire
                // globals map before reading one key. LoadGlobal semantics match
                // page-scope `variables.x` reads exactly (locals-then-globals).
                // Unsafe inside function bodies: `variables` there means the locals
                // merge or a CFC's `__variables` struct — LoadGlobal would hit page
                // globals instead. Also unsafe for null-safe `variables?.foo`.
                if !access.null_safe && self.function_depth == 0 {
                    if let Expression::Identifier(ref ident) = *access.object {
                        if ident.name.eq_ignore_ascii_case("variables") {
                            instructions.push(BytecodeOp::LoadGlobal(access.member.clone()));
                            return;
                        }
                    }
                }
                // Peephole: `<ident>.<member>` with no null-safe → fuse into
                // LoadLocalProperty. Skips the intermediate stack push of the
                // receiver plus the separate GetProperty dispatch.
                //
                // Skip when the identifier is a CFML-reserved scope name —
                // those resolve through the scope chain (globals, request,
                // __variables fallback, etc.) not just the locals map, and the
                // simple `locals.get(name)` lookup would return the wrong
                // value (typically null).
                if !access.null_safe {
                    if let Expression::Identifier(ref ident) = *access.object {
                        if !is_reserved_scope_name(&ident.name) {
                            instructions.push(BytecodeOp::LoadLocalProperty(
                                ident.name.clone(),
                                access.member.clone(),
                            ));
                            return;
                        }
                    }
                }
                // For null-safe access, use TryLoadLocal for simple identifiers
                if access.null_safe {
                    if let Expression::Identifier(ref ident) = *access.object {
                        instructions.push(BytecodeOp::TryLoadLocal(ident.name.clone()));
                    } else {
                        self.compile_expression(&access.object, instructions);
                    }
                } else {
                    self.compile_expression(&access.object, instructions);
                }
                if access.null_safe {
                    // Null-safe: if object is null, skip property access (null stays on stack)
                    // JumpIfNotNull peeks without popping, so no Dup needed
                    let jump_idx = instructions.len();
                    instructions.push(BytecodeOp::JumpIfNotNull(0)); // placeholder
                    // Object is null - it's on the stack, skip the GetProperty
                    let jump_end = instructions.len();
                    instructions.push(BytecodeOp::Jump(0)); // placeholder
                    // Object is not null - do the property access
                    instructions[jump_idx] = BytecodeOp::JumpIfNotNull(instructions.len());
                    instructions.push(BytecodeOp::GetProperty(access.member.clone()));
                    instructions[jump_end] = BytecodeOp::Jump(instructions.len());
                } else {
                    instructions.push(BytecodeOp::GetProperty(access.member.clone()));
                }
            }
            Expression::ArrayAccess(access) => {
                self.compile_expression(&access.array, instructions);
                self.compile_expression(&access.index, instructions);
                instructions.push(BytecodeOp::GetIndex);
            }
            Expression::FunctionCall(call) => {
                // Special-case: super(args) — explicit Rust-parent ctor call from
                // inside a CFC init() method. Compile args then emit a dedicated
                // op that re-runs the registered constructor and replaces
                // this.__super with the freshly-built NativeObject.
                if matches!(&*call.name, Expression::Super(_)) {
                    let n = call.arguments.len();
                    for arg in &call.arguments {
                        self.compile_expression(arg, instructions);
                    }
                    instructions.push(BytecodeOp::CallRustSuperCtor(n));
                    return;
                }
                // Special-case: isDefined("varName") -> IsDefined bytecode
                if let Expression::Identifier(ident) = &*call.name {
                    if ident.name.to_lowercase() == "isdefined" && call.arguments.len() == 1 {
                        if let Expression::Literal(Literal { value: LiteralValue::String(ref var_name), .. }) = call.arguments[0] {
                            instructions.push(BytecodeOp::IsDefined(var_name.clone()));
                            return;
                        }
                    }
                    // Special-case: isNull(varName) -> TryLoadLocal + IsNull
                    // Uses TryLoadLocal so undefined vars return Null (true) rather than erroring
                    if ident.name.to_lowercase() == "isnull" && call.arguments.len() == 1 {
                        if let Expression::Identifier(ref arg_ident) = call.arguments[0] {
                            instructions.push(BytecodeOp::TryLoadLocal(arg_ident.name.clone()));
                            instructions.push(BytecodeOp::IsNull);
                            return;
                        }
                    }
                }

                let has_spread = call.arguments.iter().any(|a| matches!(a, Expression::Spread(_)));
                let has_named = call.arguments.iter().any(|a| matches!(a, Expression::NamedArgument(_)));
                if has_spread {
                    // Push function reference first
                    if let Expression::Identifier(ident) = &*call.name {
                        instructions.push(BytecodeOp::LoadGlobal(ident.name.clone()));
                    } else {
                        self.compile_expression(&call.name, instructions);
                    }
                    // Build args array using concat pattern
                    instructions.push(BytecodeOp::BuildArray(0));
                    for arg in &call.arguments {
                        if let Expression::Spread(inner) = arg {
                            self.compile_expression(inner, instructions);
                            instructions.push(BytecodeOp::ConcatArrays);
                        } else {
                            self.compile_expression(arg, instructions);
                            instructions.push(BytecodeOp::BuildArray(1));
                            instructions.push(BytecodeOp::ConcatArrays);
                        }
                    }
                    instructions.push(BytecodeOp::CallSpread);
                } else if has_named {
                    // Named arguments: push function ref, then compile values, emit CallNamed
                    if let Expression::Identifier(ident) = &*call.name {
                        instructions.push(BytecodeOp::LoadGlobal(ident.name.clone()));
                    } else {
                        self.compile_expression(&call.name, instructions);
                    }
                    let mut names = Vec::new();
                    for arg in &call.arguments {
                        if let Expression::NamedArgument(named) = arg {
                            names.push(named.name.clone());
                            self.compile_expression(&named.value, instructions);
                        } else {
                            // Positional arg mixed with named — use empty name
                            names.push(String::new());
                            self.compile_expression(arg, instructions);
                        }
                    }
                    instructions.push(BytecodeOp::CallNamed(names, call.arguments.len()));
                } else {
                    // Push function reference first
                    if let Expression::Identifier(ident) = &*call.name {
                        instructions.push(BytecodeOp::LoadGlobal(ident.name.clone()));
                    } else {
                        self.compile_expression(&call.name, instructions);
                    }
                    // Push arguments
                    for arg in &call.arguments {
                        self.compile_expression(arg, instructions);
                    }
                    instructions.push(BytecodeOp::Call(call.arguments.len()));
                }
            }
            Expression::MethodCall(call) => {
                // Determine write-back target from the AST.
                // this.items.append(x) → write_back = Some(("this", Some("items")))
                // dog.method(x)        → write_back = Some(("dog", None))
                let write_back = Self::method_call_write_back(&call.object);
                let has_named = call
                    .arguments
                    .iter()
                    .any(|arg| matches!(arg, Expression::NamedArgument(_)));
                let compile_args =
                    |compiler: &mut Self, instructions: &mut Vec<BytecodeOp>| -> Vec<String> {
                        let mut names = Vec::with_capacity(call.arguments.len());
                        for arg in &call.arguments {
                            if let Expression::NamedArgument(named) = arg {
                                names.push(named.name.clone());
                                compiler.compile_expression(&named.value, instructions);
                            } else {
                                names.push(String::new());
                                compiler.compile_expression(arg, instructions);
                            }
                        }
                        names
                    };

                // For null-safe method calls, use TryLoadLocal for simple identifiers
                if call.null_safe {
                    if let Expression::Identifier(ref ident) = *call.object {
                        instructions.push(BytecodeOp::TryLoadLocal(ident.name.clone()));
                    } else {
                        self.compile_expression(&call.object, instructions);
                    }
                } else {
                    self.compile_expression(&call.object, instructions);
                }
                if call.null_safe {
                    let jump_idx = instructions.len();
                    instructions.push(BytecodeOp::JumpIfNotNull(0));
                    let jump_end = instructions.len();
                    instructions.push(BytecodeOp::Jump(0));
                    instructions[jump_idx] = BytecodeOp::JumpIfNotNull(instructions.len());
                    let names = compile_args(self, instructions);
                    if has_named {
                        instructions.push(BytecodeOp::CallMethodNamed(
                            call.method.clone(),
                            names,
                            call.arguments.len(),
                            write_back.clone(),
                        ));
                    } else {
                        instructions.push(BytecodeOp::CallMethod(
                            call.method.clone(),
                            call.arguments.len(),
                            write_back.clone(),
                        ));
                    }
                    instructions[jump_end] = BytecodeOp::Jump(instructions.len());
                } else {
                    let names = compile_args(self, instructions);
                    if has_named {
                        instructions.push(BytecodeOp::CallMethodNamed(
                            call.method.clone(),
                            names,
                            call.arguments.len(),
                            write_back,
                        ));
                    } else {
                        instructions.push(BytecodeOp::CallMethod(
                            call.method.clone(),
                            call.arguments.len(),
                            write_back,
                        ));
                    }
                }
            }
            Expression::Array(arr) => {
                let has_spread = arr.elements.iter().any(|e| matches!(e, Expression::Spread(_)));
                if has_spread {
                    // Start with empty array
                    instructions.push(BytecodeOp::BuildArray(0));
                    for elem in &arr.elements {
                        if let Expression::Spread(inner) = elem {
                            // Compile spread expr (should be array), concat
                            self.compile_expression(inner, instructions);
                            instructions.push(BytecodeOp::ConcatArrays);
                        } else {
                            // Compile single element, wrap in 1-element array, concat
                            self.compile_expression(elem, instructions);
                            instructions.push(BytecodeOp::BuildArray(1));
                            instructions.push(BytecodeOp::ConcatArrays);
                        }
                    }
                } else {
                    for elem in &arr.elements {
                        self.compile_expression(elem, instructions);
                    }
                    instructions.push(BytecodeOp::BuildArray(arr.elements.len()));
                }
            }
            Expression::Struct(st) => {
                let has_spread = st.pairs.iter().any(|(k, _)| matches!(k, Expression::Spread(_)));
                if has_spread {
                    // Start with empty struct
                    instructions.push(BytecodeOp::BuildStruct(0));
                    for (key, value) in &st.pairs {
                        if let Expression::Spread(_inner) = key {
                            // Spread: compile the value (which is the spread expr), merge
                            self.compile_expression(value, instructions);
                            instructions.push(BytecodeOp::MergeStructs);
                        } else {
                            // Normal pair: compile key/value, build 1-pair struct, merge
                            match key {
                                Expression::Identifier(ident) => {
                                    instructions.push(BytecodeOp::String(ident.name.clone()));
                                }
                                _ => {
                                    self.compile_expression(key, instructions);
                                }
                            }
                            self.compile_expression(value, instructions);
                            instructions.push(BytecodeOp::BuildStruct(1));
                            instructions.push(BytecodeOp::MergeStructs);
                        }
                    }
                } else {
                    for (key, value) in &st.pairs {
                        match key {
                            Expression::Identifier(ident) => {
                                instructions.push(BytecodeOp::String(ident.name.clone()));
                            }
                            _ => {
                                self.compile_expression(key, instructions);
                            }
                        }
                        self.compile_expression(value, instructions);
                    }
                    instructions.push(BytecodeOp::BuildStruct(st.pairs.len()));
                }
            }
            Expression::Ternary(tern) => {
                self.compile_expression(&tern.condition, instructions);
                let jump_false = instructions.len();
                instructions.push(BytecodeOp::JumpIfFalse(0));

                self.compile_expression(&tern.then_expr, instructions);
                let jump_end = instructions.len();
                instructions.push(BytecodeOp::Jump(0));

                instructions[jump_false] = BytecodeOp::JumpIfFalse(instructions.len());
                self.compile_expression(&tern.else_expr, instructions);
                instructions[jump_end] = BytecodeOp::Jump(instructions.len());
            }
            Expression::New(new_expr) => {
                // Parser may parse `new Dog(args)` as class=FunctionCall(Dog, args)
                // Extract the class name and push it for VM resolution
                match &*new_expr.class {
                    Expression::FunctionCall(call) => {
                        // Try flattening dot-path: new a.b.c(args) parses as FunctionCall(MemberAccess(a,b).c, args)
                        if let Some(path) = Self::flatten_member_access(&call.name) {
                            instructions.push(BytecodeOp::String(path));
                        } else if let Expression::Identifier(ident) = &*call.name {
                            instructions.push(BytecodeOp::String(ident.name.clone()));
                        } else {
                            self.compile_expression(&call.name, instructions);
                        }
                        for arg in &call.arguments {
                            self.compile_expression(arg, instructions);
                        }
                        instructions.push(BytecodeOp::NewObject(call.arguments.len()));
                    }
                    Expression::Identifier(ident) => {
                        // Push class name as string - VM will look up in locals, globals, or .cfc files
                        instructions.push(BytecodeOp::String(ident.name.clone()));
                        for arg in &new_expr.arguments {
                            self.compile_expression(arg, instructions);
                        }
                        instructions.push(BytecodeOp::NewObject(new_expr.arguments.len()));
                    }
                    Expression::MemberAccess(_) => {
                        // Handle bare dotted path: new a.b.c without parens
                        if let Some(path) = Self::flatten_member_access(&new_expr.class) {
                            instructions.push(BytecodeOp::String(path));
                        } else {
                            self.compile_expression(&new_expr.class, instructions);
                        }
                        for arg in &new_expr.arguments {
                            self.compile_expression(arg, instructions);
                        }
                        instructions.push(BytecodeOp::NewObject(new_expr.arguments.len()));
                    }
                    _ => {
                        self.compile_expression(&new_expr.class, instructions);
                        for arg in &new_expr.arguments {
                            self.compile_expression(arg, instructions);
                        }
                        instructions.push(BytecodeOp::NewObject(new_expr.arguments.len()));
                    }
                }
            }
            Expression::Closure(closure) => {
                // Compile closure body into separate function.
                // Lucee: closure inherits its enclosing function's localMode
                // when it doesn't carry its own attribute. Track current_fn for
                // nested closures-inside-closures too.
                let closure_declared = metadata_declared_local_mode(&closure.metadata);
                let effective_declared = closure_declared.or(self.current_fn_local_mode);
                let prev_fn_local_mode = self.current_fn_local_mode;
                self.current_fn_local_mode = effective_declared;

                let mut func_instructions = Vec::new();
                // Emit default parameter value preamble for closures
                for param in &closure.params {
                    if let Some(ref default_expr) = param.default {
                        func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                        func_instructions.push(BytecodeOp::IsNull);
                        let jump_idx = func_instructions.len();
                        func_instructions.push(BytecodeOp::JumpIfFalse(0));
                        self.compile_expression(default_expr, &mut func_instructions);
                        func_instructions.push(BytecodeOp::StoreLocal(param.name.clone()));
                        // Also update the arguments scope
                        func_instructions.push(BytecodeOp::LoadLocal("arguments".to_string()));
                        func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                        func_instructions.push(BytecodeOp::SetProperty(param.name.clone()));
                        func_instructions.push(BytecodeOp::StoreLocal("arguments".to_string()));
                        func_instructions[jump_idx] = BytecodeOp::JumpIfFalse(func_instructions.len());
                    }
                }
                for s in &closure.body {
                    self.compile_statement(s, &mut func_instructions);
                }
                func_instructions.push(BytecodeOp::Null);
                func_instructions.push(BytecodeOp::Return);

                let func_name = format!("__closure_{}", self.program.functions.len());
                let bc_func = BytecodeFunction {
                    name: func_name.clone(),
                    params: closure.params.iter().map(|p| p.name.clone()).collect(),
                    required_params: closure.params.iter().map(|p| p.required).collect(),
                    instructions: func_instructions,
                    source_file: None,
                    declared_local_mode: effective_declared,
                };

                let func_idx = self.program.functions.len();
                self.program.functions.push(Arc::new(bc_func));
                instructions.push(BytecodeOp::DefineFunction(func_idx));
                self.current_fn_local_mode = prev_fn_local_mode;
            }
            Expression::ArrowFunction(arrow) => {
                // Arrow functions inherit enclosing function's mode too
                // (they have no attribute syntax of their own).
                let arrow_effective = self.current_fn_local_mode;
                let prev_fn_local_mode = self.current_fn_local_mode;
                self.current_fn_local_mode = arrow_effective;
                let mut func_instructions = Vec::new();
                // Emit default parameter value preamble for arrow functions
                for param in &arrow.params {
                    if let Some(ref default_expr) = param.default {
                        func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                        func_instructions.push(BytecodeOp::IsNull);
                        let jump_idx = func_instructions.len();
                        func_instructions.push(BytecodeOp::JumpIfFalse(0));
                        self.compile_expression(default_expr, &mut func_instructions);
                        func_instructions.push(BytecodeOp::StoreLocal(param.name.clone()));
                        // Also update the arguments scope
                        func_instructions.push(BytecodeOp::LoadLocal("arguments".to_string()));
                        func_instructions.push(BytecodeOp::LoadLocal(param.name.clone()));
                        func_instructions.push(BytecodeOp::SetProperty(param.name.clone()));
                        func_instructions.push(BytecodeOp::StoreLocal("arguments".to_string()));
                        func_instructions[jump_idx] = BytecodeOp::JumpIfFalse(func_instructions.len());
                    }
                }
                self.compile_expression(&arrow.body, &mut func_instructions);
                func_instructions.push(BytecodeOp::Return);

                let func_name = format!("__arrow_{}", self.program.functions.len());
                let bc_func = BytecodeFunction {
                    name: func_name.clone(),
                    params: arrow.params.iter().map(|p| p.name.clone()).collect(),
                    required_params: arrow.params.iter().map(|p| p.required).collect(),
                    instructions: func_instructions,
                    source_file: None,
                    declared_local_mode: arrow_effective,
                };

                let func_idx = self.program.functions.len();
                self.program.functions.push(Arc::new(bc_func));
                instructions.push(BytecodeOp::DefineFunction(func_idx));
                self.current_fn_local_mode = prev_fn_local_mode;
            }
            Expression::This(_) => {
                instructions.push(BytecodeOp::LoadLocal("this".to_string()));
            }
            Expression::Super(_) => {
                instructions.push(BytecodeOp::LoadLocal("this".to_string()));
                instructions.push(BytecodeOp::GetProperty("__super".to_string()));
            }
            Expression::StringInterpolation(interp) => {
                if interp.parts.is_empty() {
                    instructions.push(BytecodeOp::String(String::new()));
                } else {
                    // Compile first part
                    self.compile_expression(&interp.parts[0], instructions);
                    // Convert to string via Concat with empty string if needed
                    if !matches!(&interp.parts[0], Expression::Literal(Literal { value: LiteralValue::String(_), .. })) {
                        instructions.push(BytecodeOp::String(String::new()));
                        instructions.push(BytecodeOp::Concat);
                    }
                    // Concat remaining parts
                    for part in &interp.parts[1..] {
                        self.compile_expression(part, instructions);
                        instructions.push(BytecodeOp::Concat);
                    }
                }
            }
            Expression::Elvis(elvis) => {
                // Elvis operator: left ?: right
                // Eval left, if not null use it, otherwise eval right
                // JumpIfNotNull peeks without popping, so no Dup needed
                // Use TryLoadLocal for simple identifiers (undefined vars → Null, not error)
                if let Expression::Identifier(ref ident) = *elvis.left {
                    instructions.push(BytecodeOp::TryLoadLocal(ident.name.clone()));
                } else {
                    self.compile_expression(&elvis.left, instructions);
                }
                let jump_idx = instructions.len();
                instructions.push(BytecodeOp::JumpIfNotNull(0)); // placeholder
                // Left is null, pop the null and eval right
                instructions.push(BytecodeOp::Pop);
                self.compile_expression(&elvis.right, instructions);
                instructions[jump_idx] = BytecodeOp::JumpIfNotNull(instructions.len());
            }
            Expression::NamedArgument(named) => {
                // Named arguments are handled at the call site; if we get here
                // in a non-call context, just compile the value
                self.compile_expression(&named.value, instructions);
            }
            Expression::Spread(inner) => {
                // Spread in a general context just compiles the inner expression
                self.compile_expression(inner, instructions);
            }
            Expression::Empty => {
                instructions.push(BytecodeOp::Null);
            }
            _ => {
                instructions.push(BytecodeOp::Null);
            }
        }
    }
}

impl Default for CfmlCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod size_probe {
    //! PR-0 size probe (RustCFML performance plan). `BytecodeOp` is the icache
    //! cost: a 500-instruction function body at 64 B is 32 KB; shrinking the op
    //! (interned `u32` identifier ids instead of `String` payloads) targets L1.
    //!
    //! Run with: `cargo test -p cfml-codegen size_probe -- --nocapture`
    use super::*;
    use std::mem::size_of;

    #[test]
    fn report_sizes() {
        let op = size_of::<BytecodeOp>();
        eprintln!("size_of::<BytecodeOp>() = {op} B");
        assert!(
            op <= 64,
            "BytecodeOp grew to {op} B (ceiling 64 B) — a perf regression. \
             If intentional, justify and raise the ceiling."
        );
    }
}
