<cfscript>
suiteBegin("Core: isInstanceOf on native (non-component) values");

// ============================================================
// Background (GH #228)
// ============================================================
// isInstanceOf() only inspected CfmlValue::Struct component metadata, so any
// non-component value (array, struct-as-map, query, primitive) always returned
// false — no matter the type name. Lucee/ACF treat CFML natives as their Java
// identities (arrays are java.util.List, structs are java.util.Map, ...), which
// TestBox's toBeInstanceOf() matcher relies on. isInstanceOf now consults the
// same native class mapping getClass().getName() already uses.
// ============================================================

arr = [ 1, 2, 3 ];
assert("array is Array", isInstanceOf(arr, "Array"), true);
assert("array is java.util.List", isInstanceOf(arr, "java.util.List"), true);
assert("array is lucee.runtime.type.ArrayImpl", isInstanceOf(arr, "lucee.runtime.type.ArrayImpl"), true);
assert("array is NOT a Struct", isInstanceOf(arr, "Struct"), false);
assert("array is NOT java.util.Map", isInstanceOf(arr, "java.util.Map"), false);

st = { a = 1 };
assert("struct is Struct", isInstanceOf(st, "Struct"), true);
assert("struct is java.util.Map", isInstanceOf(st, "java.util.Map"), true);
assert("struct is NOT java.util.List", isInstanceOf(st, "java.util.List"), false);

s = "hello";
assert("string is String", isInstanceOf(s, "String"), true);
assert("string is java.lang.String", isInstanceOf(s, "java.lang.String"), true);

b = true;
assert("boolean is Boolean", isInstanceOf(b, "Boolean"), true);
assert("boolean is java.lang.Boolean", isInstanceOf(b, "java.lang.Boolean"), true);

n = 42;
assert("integer is numeric", isInstanceOf(n, "numeric"), true);
assert("integer is java.lang.Integer", isInstanceOf(n, "java.lang.Integer"), true);

d = 3.14;
assert("double is numeric", isInstanceOf(d, "numeric"), true);
assert("double is java.lang.Double", isInstanceOf(d, "java.lang.Double"), true);

// negative control: nonsense type name never matches
assert("array is NOT com.example.Nope", isInstanceOf(arr, "com.example.Nope"), false);

suiteEnd();
</cfscript>
