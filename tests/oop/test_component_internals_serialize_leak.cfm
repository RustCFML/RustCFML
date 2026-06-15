<cfscript>
suiteBegin("OOP: component iteration / serialization must not leak engine-internal keys");

// Background: iterating a component (for(k in obj)) or SerializeJSON(obj) must
// expose only its data members — never engine-internal bookkeeping keys. On
// Lucee/ACF/BoxLang for-in yields the public members (and functions) but NO
// engine internals, and SerializeJSON(component) emits only the data
// properties (no functions, no internals). RustCFML 0.161.0 leaks
// __name / __source_file / __variables into both for-in and SerializeJSON,
// and SerializeJSON additionally emits every method as a null-valued key.
//
//   o = new Probe(); o.id = 7; o.title = "Hello";
//   for (k in o) ...          RustCFML: __name,greet,__source_file,__variables,id,title
//                             Lucee:    ID,GREET,TITLE   (no __* internals)
//   serializeJSON(o)          RustCFML: {"__name":..,"greet":null,"__source_file":..,"__variables":{..},"id":7,"title":"Hello"}
//                             Lucee:    {"ID":7,"TITLE":"Hello"}
//
// Why it matters for Wheels: model/properties.cfc::properties() does
// for(local.key in this) to collect a model's properties, and the canonical
// REST pattern renderWith(data=modelObject) runs SerializeJSON on the model.
// On RustCFML a single-record JSON response comes back with ~379 keys of
// engine internals + null methods instead of the model's columns.

slpObj = createObject("component", "oop.SerializeLeakProbe");
slpObj.id = 7;
slpObj.title = "Hello";

// --- for-in must not yield engine-internal __* keys ---
slpKeys = "";
for (slpK in slpObj) { slpKeys = listAppend(slpKeys, slpK); }
assertFalse("for-in over a component does not yield __name", listFindNoCase(slpKeys, "__name") gt 0);
assertFalse("for-in over a component does not yield __source_file", listFindNoCase(slpKeys, "__source_file") gt 0);
assertFalse("for-in over a component does not yield __variables", listFindNoCase(slpKeys, "__variables") gt 0);
assertTrue("for-in over a component DOES yield its data members", listFindNoCase(slpKeys, "id") gt 0 && listFindNoCase(slpKeys, "title") gt 0);

// --- SerializeJSON of a component must emit only data, no internals/functions ---
slpJson = serializeJSON(slpObj);
assertFalse("serializeJSON(component) omits __name", findNoCase("__name", slpJson) gt 0);
assertFalse("serializeJSON(component) omits __source_file", findNoCase("__source_file", slpJson) gt 0);
assertFalse("serializeJSON(component) omits __variables", findNoCase("__variables", slpJson) gt 0);
assertFalse("serializeJSON(component) omits method members", findNoCase("greet", slpJson) gt 0);
assertTrue("serializeJSON(component) includes data members", findNoCase("Hello", slpJson) gt 0);

suiteEnd();
</cfscript>
