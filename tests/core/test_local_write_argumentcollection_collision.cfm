<cfscript>
suiteBegin("Core: local.X write survives an ancestor argumentCollection key colliding case-insensitively (GH ##243)");

// ============================================================
// Background (GH ##243)
// ============================================================
// A `local.X = ...` assignment must ALWAYS claim a frame-local slot, regardless
// of any `argumentCollection` contents anywhere in the call chain. v0.418.0
// regressed this: `DeclareLocal` recorded both casings of the name in
// declared_locals but only removed the EXACT-cased key from the
// inherited-keys set. When an ancestor call hopped via
// `argumentCollection = arguments` carrying an overflow arg whose name
// case-insensitively matched X (e.g. `filename` vs a deeper `local.fileName`),
// the inherited lowercase key stayed flagged, the store wrote into it via a
// case-insensitive insert, and the local-scope view filtered the write straight
// back out — so `StructKeyExists(local, "fileName")` was false and a later read
// threw `Variable 'fileName' is undefined`. This boot-broke the Wheels suite.

function inner(required struct properties) {
	local.fileName = "OK";
	if (StructKeyExists(local, "fileName")) {
		return "kept: " & local.fileName;
	}
	return "LOCAL WRITE LOST";
}
function middle(struct properties = {}) {
	return inner(properties = arguments.properties);
}
function outer(struct properties = {}) {
	return middle(argumentCollection = arguments);
}

// (a) The failing case: an overflow named arg (`filename`) enters via an
//     argumentCollection hop, colliding case-insensitively with a deeper
//     `local.fileName`.
assert("overflow key collides w/ deeper local (diff casing)", outer(filename = "collide"), "kept: OK");

// (b) A clean overflow arg with no collision was always fine.
assert("clean overflow arg", outer(title = "clean"), "kept: OK");

// (c) Calling inner directly (no AC hop) was always fine.
assert("direct call, colliding key in struct", inner({filename: "collide"}), "kept: OK");

// (d) The written value must actually be readable (not just exist).
function reader(required struct properties) {
	local.fileName = "VALUE-42";
	return local.fileName;
}
function passer(struct properties = {}) {
	return reader(properties = arguments.properties);
}
function top(struct properties = {}) {
	return passer(argumentCollection = arguments);
}
assert("local read after collision", top(FILENAME = "x"), "VALUE-42");

suiteEnd();
</cfscript>
