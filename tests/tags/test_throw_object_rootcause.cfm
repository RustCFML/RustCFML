<cfscript>
suiteBegin("throw object / extendedInfo / rootCause");

// --- throw(...) call form preserves extendedInfo ---
extErr = "";
try {
	throw(type = "Custom.T1", message = "m1", extendedInfo = "extra-info");
} catch (any e) {
	extErr = e;
}
assert("throw extendedInfo preserved", extErr.extendedInfo, "extra-info");

// --- throw(object=...) re-throws the caught exception verbatim ---
orig = "";
try {
	throw(type = "Custom.T2", message = "m2", detail = "d2", extendedInfo = "ext2");
} catch (any e) {
	orig = e;
}
rethrown = "";
try {
	throw(object = orig);
} catch (any e) {
	rethrown = e;
}
assert("throw object preserves message", rethrown.message, "m2");
assert("throw object preserves type", rethrown.type, "Custom.T2");
assert("throw object preserves detail", rethrown.detail, "d2");
assert("throw object preserves extendedInfo", rethrown.extendedInfo, "ext2");

// --- explicit attrs override the object ---
// A DELIBERATE divergence, so guarded rather than reported as a Lucee failure
// (GH #352, known-issues.md §60). We merge: the object supplies the base and an
// explicit attribute overrides it. Lucee's `Throw.java` instead processes
// `message` BEFORE `object` and throws a fresh exception built only from the
// tag's own attributes, so supplying `message=` there discards the object
// entirely — the type resets to `application` and the detail is lost.
// `throw( object=e )` on its own is identical on both engines and is asserted
// unguarded above.
if ( isRustCFML() ) {
	override = "";
	try {
		throw(object = orig, message = "overridden");
	} catch (any e) {
		override = e;
	}
	assert("throw object message override", override.message, "overridden");
	assert("throw object keeps type under override", override.type, "Custom.T2");
	assert("throw object keeps detail under override", override.detail, "d2");

	// The mirror case: an explicit `type=` alongside an object. Lucee ignores it
	// (the object wins outright); we let it override.
	typeOverride = "";
	try {
		throw(object = orig, type = "New.T");
	} catch (any e) {
		typeOverride = e;
	}
	assert("throw object type override", typeOverride.type, "New.T");
	assert("throw object keeps message under type override", typeOverride.message, "m2");
}

// --- plain throw still works ---
plain = "";
try {
	throw(message = "plain");
} catch (any e) {
	plain = e;
}
assert("plain throw message", plain.message, "plain");

// --- every exception carries a rootCause ---
// A RustCFML/ACF extension, NOT Lucee parity: measured on Lucee 7.1.0.204,
// cfcatch has no rootCause key at all — not for a plain throw, not for an
// engine error, not even when `cause=` is passed explicitly. The comment here
// used to claim Lucee parity; it does not hold, so these are guarded as the
// superset they are rather than reported as a Lucee failure.
rc = "";
try {
	throw(type = "Custom.T3", message = "m3", extendedInfo = "ext3");
} catch (any e) {
	rc = e;
}
if ( isRustCFML() ) {
	assertTrue("exception has rootCause", structKeyExists(rc, "rootCause"));
	assert("rootCause.type matches", rc.rootCause.type, "Custom.T3");
	assert("rootCause.message matches", rc.rootCause.message, "m3");
	assert("rootCause.extendedInfo matches", rc.rootCause.extendedInfo, "ext3");
	assertFalse("rootCause does not nest a rootCause", structKeyExists(rc.rootCause, "rootCause"));
}

// runtime errors also get a rootCause (same RustCFML/ACF extension)
divErr = "";
try {
	dummy = 1 / 0;
} catch (any e) {
	divErr = e;
}
if ( isRustCFML() ) {
	assertTrue("runtime error has rootCause", structKeyExists(divErr, "rootCause"));
}

tagObjErr = "";
tagExtErr = "";
</cfscript>

<!--- tag forms: <cfthrow object=...> and extendedInfo --->
<cftry>
	<cfthrow object="#orig#">
	<cfcatch type="any"><cfset tagObjErr = cfcatch></cfcatch>
</cftry>
<cftry>
	<cfthrow message="tagmsg" type="Custom.T4" extendedinfo="tagext">
	<cfcatch type="any"><cfset tagExtErr = cfcatch></cfcatch>
</cftry>

<cfscript>
assert("cfthrow object= preserves message", tagObjErr.message, "m2");
assert("cfthrow object= preserves type", tagObjErr.type, "Custom.T2");
assert("cfthrow extendedInfo preserved", tagExtErr.extendedInfo, "tagext");

suiteEnd();
</cfscript>
