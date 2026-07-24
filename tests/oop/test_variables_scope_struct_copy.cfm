<cfscript>
// GH #285: StructAppend/StructCopy (and member-form .append) sourced from a
// component's `variables` scope must copy ALL members, including UDF/method
// members — Lucee treats functions as ordinary struct values here. Regressed in
// v0.507.0 when the flyweight method table stopped being enumerated by the
// Struct-Struct copy path (data members kept, function members silently dropped).
suiteBegin("variables-scope struct copy keeps function members (GH ##285)");

obj = new oop.VarScopeCopyProbe();

// --- StructAppend(dest, variables) ---
a = obj.viaStructAppend();
assertTrue("StructAppend keeps public method member", a.pluralize);
assertTrue("StructAppend keeps private method member", a.secret);
assertFalse("StructAppend does not leak the 'this' self-ref", a.this_leak);
assertFalse("StructAppend does not leak the 'super' self-ref", a.super_leak);

// --- StructCopy(variables) ---
c = obj.viaStructCopy();
assertTrue("StructCopy keeps public method member", c.pluralize);
assertTrue("StructCopy keeps private method member", c.secret);

// --- member-form dest.append(variables) ---
m = obj.viaMemberAppend();
assertTrue("dest.append(variables) keeps public method member", m.pluralize);

// --- externally-returned variables reference ---
extVars = obj.exposeVars();
ext = {};
StructAppend(ext, extVars);
assertTrue("StructAppend from external variables reference keeps method", StructKeyExists(ext, "pluralize"));

// --- GH #285 secondary: missing-member CALL on a plain struct throws (no bare-
//     name ambient fallback / runaway recursion), matching the rvalue read ---
assertThrows("missing member call on a plain struct throws (not infinite recursion)",
             function(){ obj.callsMissingMemberOnPlainStruct(); });

suiteEnd();
</cfscript>
