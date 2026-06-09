<cfscript>
suiteBegin("Core: `param` usable as an ordinary identifier");

// ============================================================
// Background  (follow-on to test_param_dotted_lhs.cfm / PR #32 / #77 by bpamiri)
// ============================================================
// `param` is a SOFT keyword on Lucee 5/6/7, Adobe ColdFusion 2018-2025, and
// BoxLang: reserved only for the cfscript `param <type> name = default;`
// statement, but otherwise a legal ordinary identifier. RustCFML 0.92.0 instead
// treats a statement that *starts* with the token `param` as a cfparam
// statement unconditionally, so using `param` as an assignment target
// (`param = "x"`) or a struct loop variable that is then written to
// (`param['default'] = ...`) is a PARSE error.
//
// Disambiguation Lucee uses: `param` followed by a type/identifier is the
// cfparam statement (`param cfgA.timeout = 30`, covered by
// test_param_dotted_lhs.cfm); `param` followed directly by `=` or `[` is an
// ordinary assignment to a variable named `param`. Sibling soft keywords
// (`name`, `type`, `default`) are already accepted as identifiers on RustCFML;
// only `param` regresses.
//
// Wheels relies on exactly this. vendor/wheels/public/helpers.cfm:460 iterates
// the parsed parameter list with `param` as the loop variable and writes back
// into it:
//
//     for (param in local.rv["parameters"]) {
//         ...
//         param['default'] = application.wheels.functions[...][param.name];
//     }
//
// Because the failure is a PARSE error, the whole file errors out on RustCFML
// (the runner's try/catch reports ERROR, not a FAIL count) — that is the
// documented red form for this gap.
// ============================================================

// --- CONTROL: same shape with a non-keyword loop var passes on BOTH engines ---
// Guards the test wiring: if this fails, the assertion harness itself is broken,
// not the `param` keyword handling.
controlParams = [ {name="a", "default"="x"}, {name="b", "default"="y"} ];
controlOut = "";
for (item in controlParams) {
	item['default'] = uCase(item['default']);
	controlOut = controlOut & item.name & "=" & item['default'] & ";";
}
assert("control: loop var `item` writes back (wiring guard)", controlOut, "a=X;b=Y;");

// --- `param` as a plain assignment target and bare read ---
param = "soft-keyword-as-var";
assert("`param` is a legal assignment target and reads back", param, "soft-keyword-as-var");

// --- `param` as a for-in struct loop variable, written to inside the loop ---
// This is the verbatim vendor/wheels/public/helpers.cfm shape.
parsedParams = [ {name="a", "default"="x"}, {name="b", "default"="y"} ];
joined = "";
for (param in parsedParams) {
	if (structKeyExists(param, "default")) {
		param['default'] = uCase(param['default']);
	}
	joined = joined & param.name & "=" & param['default'] & ";";
}
assert("`param` loop var with write-back (helpers.cfm shape)", joined, "a=X;b=Y;");

// --- `param` survives as a readable value after the loop ---
assert("`param` still readable after the loop", param.name, "b");

suiteEnd();
</cfscript>
