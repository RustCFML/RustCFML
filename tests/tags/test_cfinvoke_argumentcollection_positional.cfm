<cfscript>
suiteBegin("Tags: cfinvoke argumentCollection forwards positional (numeric-keyed) args");

// ============================================================
// Background
// ============================================================
// cfinvoke's argumentCollection is a full argument collection, exactly like
// fn(argumentCollection=st) and the invoke() BIF: NUMERIC string keys ("1",
// "2", ...) are forwarded as POSITIONAL arguments and named keys as named
// arguments. Lucee, Adobe CF, and BoxLang all honor numeric keys positionally.
//
// RustCFML 0.153.0 forwards the NAMED keys but silently DROPS the numeric/
// positional keys — the callee's positional param keeps its default:
//
//   function take(any a, string named="DEF") {...}
//   cfinvoke(component=o, method="take", returnVariable="r",
//            argumentCollection = { 1: {body:"POS"}, named: "N" });
//     Lucee 5.4.8.2    -> r = "a=struct:POS|named=N"  (positional 1 -> a)
//     RustCFML 0.153.0 -> r = "a=|named=N"            (positional 1 DROPPED)
//
// Why it matters for Wheels: Global.cfc $invoke() ends in
//   cfinvoke(attributeCollection="#local.args#")
// where local.args.argumentCollection is rebuilt from the dynamic call's
// positional args as a numeric-keyed struct (Global.cfc ~289-305). It is the
// dispatch path behind onMissingMethod, so association create/new dynamic
// methods (post.createComment(struct), post.deleteAllComments() for
// dependent=delete) lose their positional argument: the foreign key (a named
// key) survives but the attributes struct (positional) is dropped, so
// validatesPresenceOf fails and dependent-cascade deletes never receive their
// args. Surfaced laddering hasMany/belongsTo associations on the blog app.
// ============================================================

cfiacObj = createObject("component", "CfInvokeArgCollPosFixture");

// --- the gap: a numeric-keyed argumentCollection entry must arrive positionally ---
cfinvoke(component = cfiacObj, method = "take", returnVariable = "local.cfiacR1",
	argumentCollection = { 1: {body: "POS"}, named: "N" });
assert("numeric-keyed argumentCollection entry forwards as the positional arg",
	local.cfiacR1, "a=struct:POS|named=N");

// --- the Wheels shape: positional struct arg + a named foreign-key-style arg ---
cfinvoke(component = cfiacObj, method = "take", returnVariable = "local.cfiacR2",
	argumentCollection = { 1: {body: "hello"}, named: "fk" });
assert("positional struct survives alongside a named key (Wheels $invoke shape)",
	local.cfiacR2, "a=struct:hello|named=fk");

// --- CONTROL (green on both engines): all-named argumentCollection ---
cfinvoke(component = cfiacObj, method = "take", returnVariable = "local.cfiacR3",
	argumentCollection = { a: {body: "NAMED"}, named: "X" });
assert("CONTROL: all-named argumentCollection forwards correctly",
	local.cfiacR3, "a=struct:NAMED|named=X");

suiteEnd();
</cfscript>
