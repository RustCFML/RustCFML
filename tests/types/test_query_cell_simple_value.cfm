<cfscript>
suiteBegin("Types: a query cell value is a simple value (IsSimpleValue + SerializeJSON)");

// Background: reading a column cell from a query (q.col, or q["col"][row]) must
// yield a SIMPLE value — IsSimpleValue() is true, and placing it into a struct/
// array that is then SerializeJSON'd preserves the value. On Lucee/ACF/BoxLang a
// query cell IS a simple string/number.
//
// RustCFML 0.161.0 returns query cells as non-simple BOXED objects:
// IsSimpleValue(q.id) is false, and SerializeJSON({v: q.id}) renders {"v":null}
// (the value is dropped). The value is still USABLE directly — it stringifies,
// participates in arithmetic, and IsNumeric() is true (PR #127 fixed the numeric
// typing) — but it is not a *simple* value, so any code that serializes a
// structure/array built from query cells silently loses the data.
//
// Why it matters: this is the single root cause behind several Wheels symptoms on
// RustCFML — renderWith(modelObject)/properties() REST output serializing as null,
// a freshly create()'d record's .id serializing null, and association/include
// columns nulling out in JSON. Wheels' demo app already works around it
// (Posts.cfc copies cells through Val()/concat; model.key() JavaCasts to int).

q = queryNew("id,name", "integer,varchar", [[1, "a"], [2, "b"]]);

// --- CONTROL (green on both engines): the value is present and usable ---
assert("CONTROL: q.id participates in arithmetic", q.id + 10, 11);
assert("CONTROL: q.id stringifies to its value", q.id & "", "1");

// --- the gap: a query cell must be a simple value ---
assertTrue("IsSimpleValue(q.id) is true for an integer query column", IsSimpleValue(q.id));
assertTrue("IsSimpleValue(q.name) is true for a varchar query column", IsSimpleValue(q.name));

// --- the gap manifests in serialization: a struct holding a query cell must not serialize to null ---
assertTrue("SerializeJSON of a struct holding q.id does not drop the value to null",
    findNoCase("null", SerializeJSON({v: q.id})) == 0);
assertTrue("SerializeJSON of a struct holding q.name does not drop the value to null",
    findNoCase("null", SerializeJSON({v: q.name})) == 0);

suiteEnd();
</cfscript>
