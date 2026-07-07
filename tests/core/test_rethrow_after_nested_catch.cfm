<cfscript>
suiteBegin("Core: rethrow re-raises the enclosing clause's exception across a nested catch (GH ##244)");

// ============================================================
// Background (GH ##244)
// ============================================================
// `rethrow` must re-raise the exception caught by ITS enclosing catch clause.
// The engine tracked the in-flight error in a single `last_exception` register,
// which a nested try/catch inside the same catch body overwrote — so `rethrow`
// re-raised the swallowed inner error (or, when the register was empty, a
// generic "No exception to rethrow"). The catch clause's own variable holds the
// full cfcatch struct; codegen now resets the register from it just before the
// Rethrow. This is the standard `catch (e) { cleanupThatMightCatch(); rethrow; }`
// rollback idiom (Wheels transaction rollback, ColdBox, etc).

// (A) inline nested try/catch inside the catch body, then rethrow
try {
	try {
		throw(type="Outer", message="outer boom");
	} catch (any e) {
		try {
			throw(type="Inner", message="inner swallowed");
		} catch (any x) {
			// swallowed on purpose
		}
		rethrow;
	}
} catch (any f) {
	assert("A: type",    f.type,    "Outer");
	assert("A: message", f.message, "outer boom");
}

// (B) the nested catch happens inside a called function
function probe() {
	try {
		throw(type="Inner", message="inner swallowed");
	} catch (any x) {
		return false;
	}
}
try {
	try {
		throw(type="Outer", message="outer boom");
	} catch (any e) {
		probe();
		rethrow;
	}
} catch (any f) {
	assert("B: type",    f.type,    "Outer");
	assert("B: message", f.message, "outer boom");
}

// (C) an inner catch's OWN rethrow still re-raises the inner exception
try {
	try {
		throw(type="Outer", message="outer");
	} catch (any e) {
		try {
			throw(type="Inner", message="inner");
		} catch (any x) {
			rethrow; // re-raise Inner
		}
	}
} catch (any f) {
	assert("C: inner clause rethrows inner", f.type, "Inner");
}

// (D) a finally that throws-and-swallows must not change the rethrown exception
try {
	try {
		throw(type="Outer", message="o");
	} catch (any e) {
		try { throw(type="Mid", message="m"); } catch (any y) {}
		rethrow;
	} finally {
		try { throw(type="Fin", message="fin"); } catch (any z) {}
	}
} catch (any f) {
	assert("D: rethrow survives swallowing finally", f.type, "Outer");
}

// (E) a plain catch/rethrow with no nesting is unchanged
try {
	try {
		throw(type="Only", message="only");
	} catch (any e) {
		rethrow;
	}
} catch (any f) {
	assert("E: plain rethrow", f.type, "Only");
}

suiteEnd();
</cfscript>
