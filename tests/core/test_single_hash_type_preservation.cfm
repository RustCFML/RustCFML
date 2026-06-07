<cfscript>
suiteBegin("Core: single hash-expression preserves type");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    On Lucee and Adobe ColdFusion, a double-quoted string whose ENTIRE content
    is a single #expression# (no literal text before, between, or after) yields
    the expression's NATIVE VALUE/TYPE -- it is NOT coerced to a string:

        st = { a: 1 };
        x  = "#st#";       // x is the STRUCT, isStruct(x) is true
        y  = "#st# ";      // y is a STRING (trailing space => concatenation)

    Moopa (and most CFML frameworks) rely on this constantly, e.g. passing a
    struct through a quoted attribute / argument:

        application.service.moo_file.upload( data = "#request.data#" )   // passes the struct

    RustCFML currently coerces the single interpolated value to a string in the
    script/expression path: "#someStruct#" becomes the lossy text
    "{a: 1}", so isStruct() is false and member access (struct.key) returns
    nothing. This breaks the framework call above (the struct arrives as a
    string, downstream reads come back empty).

    WHAT TO FIX
    -----------
    cfml-codegen compile_expression, the Expression::StringInterpolation arm
    (crates/cfml-codegen/src/compiler.rs). It always coerces the first part to a
    string (push String("") + Concat) even when there is exactly one part. When
    interp.parts.len() == 1 and that part is the interpolated expression (not a
    string literal), emit the expression's value WITHOUT the empty-string Concat,
    so the native type is preserved. Keep concatenation for the multi-part
    (mixed literal + #expr#) case -- that must still produce a string.

    Note the tag-attribute path ALREADY does this (see single_hash_expr in
    cfml-compiler/src/tag_parser.rs, used by cfparam / cfhttp / cfargument and
    exercised by the cfloop control below) -- this test pins the same rule for
    the general expression path (cfset, function args, struct/array literals,
    cfreturn).
    ============================================================
--->

<cfset g_arr = [10, 20, 30] />
<cfset g_ctrl_sum = 0 />
<cfloop array="#g_arr#" index="g_i"><cfset g_ctrl_sum += g_i /></cfloop>

<cfscript>
st = {}; st.k = "v";
arr = [10, 20, 30];

function typeOf(v) {
    if (isStruct(v))      return "struct";
    if (isArray(v))       return "array";
    if (isSimpleValue(v)) return "string";
    return "other";
}
function takeArg(x)            { return typeOf(arguments.x); }
function returnsLoneHash()     { return "#st#"; }

// ---- controls: already correct on RustCFML AND Lucee ----

bareAssign = st;
assert("control: bare assignment (no quotes) keeps struct", isStruct(bareAssign), true);

mixedText = "x #st# y";
assert("control: literal text + interpolation is a string", typeOf(mixedText), "string");

assert("control: tag attribute (cfloop array) already preserves the array",
    g_ctrl_sum, 60);

// ---- gaps: a quoted string that is EXACTLY one #expr# must keep the type ----

// 1. cfset assignment
loneStruct = "#st#";
assert("cfset: lone hash-expr preserves struct", typeOf(loneStruct), "struct");
assert("cfset: preserved struct is usable (member access)",
    typeOf(loneStruct) == "struct" ? loneStruct.k : "", "v");

loneArray = "#arr#";
assert("cfset: lone hash-expr preserves array", typeOf(loneArray), "array");
assert("cfset: preserved array length intact",
    typeOf(loneArray) == "array" ? arrayLen(loneArray) : 0, 3);

// 2. named function argument
assert("named arg: lone hash-expr preserves struct", takeArg(x="#st#"), "struct");

// 3. positional function argument
assert("positional arg: lone hash-expr preserves struct", takeArg("#st#"), "struct");

// 4. struct-literal value
svLit = { d = "#st#" };
assert("struct-literal value: lone hash-expr preserves struct", typeOf(svLit.d), "struct");

// 5. array element
aeLit = [ "#st#" ];
assert("array element: lone hash-expr preserves struct", typeOf(aeLit[1]), "struct");

// 6. cfreturn
assert("cfreturn: lone hash-expr preserves struct", typeOf(returnsLoneHash()), "struct");

suiteEnd();
</cfscript>
