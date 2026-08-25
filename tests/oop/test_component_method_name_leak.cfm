<!--- GH #360 — instantiating a component used to publish its method names into
      the VM's global user-function table, where a BARE-name read in any later
      template resolved against them. That made a CFC's private naming choices
      ambient for the rest of the request: a page reading a bare `status` could
      pick up some component's `status()` method instead of erroring, and the
      result depended on which components had run first.

      It was also self-inconsistent — structKeyExists(variables,...) and
      isDefined() both said no while a bare read said yes.

      Measured on Lucee 7.1.0.204: the bare read throws
      "variable [AVERYUNIQUEMETHODNAME360] doesn't exist". --->
<cfscript>
suiteBegin("OOP: component methods do not leak as bare names (GH ##360)");

_probe = createObject("component", "oop.MethodLeakProbe");

// The instance works normally — member dispatch is unaffected.
assert("member dispatch still works", _probe.aVeryUniqueMethodName360(), "ran");
assert("a sibling method is still callable bare from INSIDE the component",
	_probe.callsItsOwnSibling360(), "ran");

// ...but the name is not visible out here, by any of the three paths.
assertFalse("not a key in the page variables scope",
	structKeyExists(variables, "aVeryUniqueMethodName360"));
assertFalse("isDefined() says no", isDefined("aVeryUniqueMethodName360"));

_bare = "threw";
try { _bare = isCustomFunction( aVeryUniqueMethodName360 ) ? "visible" : "other"; }
catch (any e) { _bare = "threw"; }
assert("a bare-name READ agrees with isDefined() and throws", _bare, "threw");

// The same holds for a bare CALL.
_call = "threw";
try { _call = aVeryUniqueMethodName360(); } catch (any e) { _call = "threw"; }
assert("a bare CALL of the method name throws", _call, "threw");

suiteEnd();
</cfscript>
