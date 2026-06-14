// Fixture for the onMissingMethod named-argument-key test.
//
// onMissingMethod receives the call's arguments in `missingMethodArguments`.
// For a NAMED-argument call (obj.probe(label="world")) the engines key that
// struct by the argument NAMES; for a POSITIONAL call (obj.probe("x")) they
// key it by numeric position ("1", "2", ...). RustCFML 0.153.0 keys the
// named-call struct by numeric position too, losing the names — so a handler
// that reads missingMethodArguments.label sees nothing.
//
// Each probe reports the shape of missingMethodArguments back to the caller
// without depending on key ORDER (Lucee uppercases/reorders struct keys, so
// any order-sensitive assertion would false-fail on a conforming engine).
component {

	// The component declares NO `probe`/`byStatus`/etc. method, so every call
	// to one of those falls through to onMissingMethod.
	public any function onMissingMethod(
		required string missingMethodName,
		required struct missingMethodArguments
	) {
		var mma = arguments.missingMethodArguments;
		return {
			// Sorted key list so the caller can do presence checks via
			// listFindNoCase without caring about engine key order.
			keyList   = structKeyList(mma),
			count     = structCount(mma),
			// Direct by-NAME reads — these are what a real handler does
			// (arguments.status, arguments.label, ...).
			hasLabel  = structKeyExists(mma, "label"),
			labelVal  = structKeyExists(mma, "label") ? mma.label : "(absent)",
			hasExtra  = structKeyExists(mma, "extra"),
			extraVal  = structKeyExists(mma, "extra") ? mma.extra : "(absent)",
			hasStatus = structKeyExists(mma, "status"),
			statusVal = structKeyExists(mma, "status") ? mma.status : "(absent)",
			// Control: numeric key presence (the positional shape).
			has1      = structKeyExists(mma, "1"),
			val1      = structKeyExists(mma, "1") ? mma["1"] : "(absent)"
		};
	}

	// Mirrors the Wheels dynamic-scope handler shape: a scope registered as
	// scope(name="byStatus", handler="scopeByStatus") is dispatched as
	// model("Post").byStatus(status="published"), arriving here as a missing
	// method whose handler reads arguments.status BY NAME.
	public any function scopeByStatus(required string status) {
		return "where=status='" & arguments.status & "'";
	}

}
