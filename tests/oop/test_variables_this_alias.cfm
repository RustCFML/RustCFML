<cfscript>
// The live `variables.this` alias must resolve to the whole component on BOTH the
// default (marker) build and the flyweight (component-instance) build — otherwise
// `getMetadata(variablesScope.this).fullname` (Wheels Plugins.$initializeMixins)
// throws "Variable 'fullname' is undefined" and the Wheels app 500s at boot.
suiteBegin("variables.this alias — component identity (Wheels boot regression)");

obj = new oop.ThisAliasProbe();

// --- self-introspection (the exact failing read path) ---
self = obj.introspectSelf();
assertTrue("structKeyExists(variables,'this')", self.hasThisKey);
assertTrue("isObject(variables.this)", self.isObj);
assertTrue("getMetadata(variables.this) has name", self.hasName);
assertTrue("getMetadata(variables.this) has fullname", self.hasFullname);
assertTrue("name identifies the component", findNoCase("ThisAliasProbe", self.name) gt 0);
assertTrue("fullname identifies the component", findNoCase("ThisAliasProbe", self.fullname) gt 0);

// --- cross-object read (Wheels $initializeMixins(variables) pattern) ---
r = obj.introspectViaReader();
assertTrue("cross-object structKeyExists(vs,'this')", r.hasThisKey);
assertTrue("cross-object isObject(vs.this)", r.isObj);
assertTrue("cross-object getMetadata has name", r.hasName);
assertTrue("cross-object getMetadata has fullname", r.hasFullname);
assertTrue("cross-object name identifies the component", findNoCase("ThisAliasProbe", r.name) gt 0);

// --- mixin injection via StructAppend(variables.this, ...) (Wheels Plugins.cfc:821):
//     the appended method must be callable on the public object ---
obj.injectMixin();
assert("StructAppend(variables.this, {fn}) injects a callable public method",
       obj.injectedMixin(), "mixed-in");

suiteEnd();
</cfscript>
