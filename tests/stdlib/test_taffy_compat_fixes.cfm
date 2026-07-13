<cfscript>
// Regression tests for the engine gaps surfaced by the Taffy 4.0.0 test suite
// (github.com/atuttle/Taffy). Each behaviour verified against Lucee 7.0.4.
suiteBegin("Taffy compatibility fixes");

// ---------------------------------------------------------------------------
// query.getColumnList(boolean upperCase)
//   Lucee member fn: true → uppercase (== columnList), false → original case.
//   Taffy's qToArray/qToStruct call getColumnList(false) on the Lucee branch;
//   before the fix it returned "" so every query→struct conversion was empty.
// ---------------------------------------------------------------------------
q = queryNew("id,Name,Email", "integer,varchar,varchar");
queryAddRow(q);
querySetCell(q, "id", 1);
querySetCell(q, "Name", "John Doe");
querySetCell(q, "Email", "john@example.com");

assert("getColumnList(false) keeps original case", q.getColumnList(false), "id,Name,Email");
assert("getColumnList(true) uppercases", q.getColumnList(true), "ID,NAME,EMAIL");
// No-arg defaults leniently to uppercase (Lucee throws; we accept it).
assert("getColumnList() defaults to uppercase", q.getColumnList(), "ID,NAME,EMAIL");

// A qToArray-style conversion built from getColumnList(false) must populate keys.
cols = listToArray(q.getColumnList(false));
row = {};
for (c in cols) { row[c] = q[c][1]; }
assert("converted row exposes id", row.id, 1);
assert("converted row exposes name", row.name, "John Doe");
assert("converted row exposes email", row.email, "john@example.com");

// ---------------------------------------------------------------------------
// binary.equals(other) — value equality (see docs/known-issues.md; Lucee uses
// Java byte[] reference identity, which RustCFML's value-clone model can't keep,
// so we compare by value — what TestBox's equalize() relies on for binaries).
// ---------------------------------------------------------------------------
a = toBinary(toBase64("test data"));
b = toBinary(toBase64("test data"));
d = toBinary(toBase64("different"));
assertTrue("binary.equals same bytes", a.equals(b));
assertFalse("binary.equals different bytes", a.equals(d));

// ---------------------------------------------------------------------------
// cfthrow(...) — the cfXXX() script alias for <cfthrow>, plus attributeCollection
// expansion (Taffy's Factory: cfthrow(attributecollection=arguments)).
// ---------------------------------------------------------------------------
assertThrows("cfthrow with named attrs throws", function() {
    cfthrow(type = "My.Type", message = "boom");
});

caught = {};
try {
    cfthrow(type = "Taffy.Factory.BeanNotFound", message = "not found");
} catch (any e) {
    caught.type = e.type;
    caught.message = e.message;
}
assert("cfthrow sets type", caught.type, "Taffy.Factory.BeanNotFound");
assert("cfthrow sets message", caught.message, "not found");

// attributeCollection spreads a runtime struct into the throw attributes.
caughtColl = {};
try {
    attrs = { type = "Via.Collection", message = "from collection" };
    cfthrow(attributecollection = attrs);
} catch (any e) {
    caughtColl.type = e.type;
    caughtColl.message = e.message;
}
assert("cfthrow attributecollection sets type", caughtColl.type, "Via.Collection");
assert("cfthrow attributecollection sets message", caughtColl.message, "from collection");

// throw(attributecollection=…) — the same expansion on the bare `throw` form.
caughtThrowColl = {};
try {
    a2 = { type = "Throw.Coll", message = "tc" };
    throw(attributecollection = a2);
} catch (any e) {
    caughtThrowColl.type = e.type;
    caughtThrowColl.message = e.message;
}
assert("throw attributecollection sets type", caughtThrowColl.type, "Throw.Coll");

// An explicit attribute overrides the collection (Lucee precedence).
caughtOverride = {};
try {
    a3 = { type = "Coll.T", message = "collection message" };
    cfthrow(message = "explicit wins", attributecollection = a3);
} catch (any e) {
    caughtOverride.type = e.type;
    caughtOverride.message = e.message;
}
assert("explicit attr overrides collection message", caughtOverride.message, "explicit wins");
assert("collection fills the un-supplied type", caughtOverride.type, "Coll.T");

suiteEnd();
</cfscript>
