<cfscript>
suiteBegin("See-through UDF variables scope (GH ##259)");

// A UDF that is NOT a component method is "see-through" (Lucee/BoxLang): its
// `variables` scope resolves to the CALLER's variables at invocation time —
// NOT to a detached/empty scope, and NOT to the scope where it was defined.
// Confirmed against Lucee 7 with this exact mod1/mod2/vh fixture set:
//   callFromMod1 (bare, defining page)      => ONE  (caller == mod1)
//   callFromMod2_ref (via stored reference) => TWO  (caller == mod2)
// Railo UDFImpl._call never swaps the variables scope on a plain UDF; BoxLang's
// FunctionBoxContext defers scope resolution to the parent context unless the
// function is executing inside a class. RustCFML previously ran a UDF invoked
// via a stored reference (`request.helperRef()`) fully detached, so
// `variables.controller` read MISSING instead of the caller's value — the root
// cause of the Preside admin sitetree "getController() null" 500.

// mod1 sets variables.controller = "ONE", includes the shared <cffunction>,
// stashes a reference to it in the request scope, and calls it bare.
module template="gh259_mod1.cfm";
// mod2 sets variables.controller = "TWO" and calls the SAME function through
// the stored reference. Being see-through, it must read mod2's "TWO".
module template="gh259_mod2.cfm";

assert( "bare call in defining module sees its own variables", request.gh259_callFromMod1, "ONE" );
assert( "reference call sees the CALLER's variables (not detached)", request.gh259_callFromMod2_ref, "TWO" );

suiteEnd();
</cfscript>
