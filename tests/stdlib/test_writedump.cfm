<cfscript>
suiteBegin("writeDump rendering");

// Capture writeDump output via cfsavecontent. In the CLI test runner this is
// the plain-text tree (web HTML output is exercised in serve mode separately).
data = { name: "RustCFML", version: 220, tags: ["a","b"], nested: { x: 1 } };

savecontent variable="dumped" {
    writeDump(data);
}

assertTrue("dump labels a struct", findNoCase("Struct", dumped) GT 0);
assertTrue("dump shows a key", findNoCase("name", dumped) GT 0);
assertTrue("dump shows a value", findNoCase("RustCFML", dumped) GT 0);
assertTrue("dump nests arrays", findNoCase("Array", dumped) GT 0);

// Labelled dump (script form) emits the label.
savecontent variable="labelled" {
    writeDump(var=data, label="My Label");
}
assertTrue("dump emits label", findNoCase("My Label", labelled) GT 0);

// writeDump of a scalar does not throw and renders the value.
savecontent variable="scalar" {
    writeDump("hello");
}
assertTrue("scalar dump renders value", findNoCase("hello", scalar) GT 0);

// Java shim objects render as "Java <class>" not a raw struct of __ markers.
shim = createObject("java", "java.util.Date").init(0);
savecontent variable="shimdump" {
    writeDump(shim);
}
// RustCFML's dump markup (Lucee renders a java.util.Date as a plain date).
if ( isRustCFML() ) {
	assertTrue("java shim labelled Java", findNoCase("Java", shimdump) GT 0);
	assertTrue("java shim shows class", findNoCase("java.util.date", shimdump) GT 0);
}
assertFalse("java shim hides __java_shim marker", findNoCase("__java_shim", shimdump) GT 0);

// output="console" sends the dump to the server console (stdout), NOT the page
// (issue 207). Captured page output must be empty, while a default dump is not.
savecontent variable="consoleDump" {
    writeDump(var="rcf207-console-marker", output="console");
}
assert("output=console emits nothing to the page", trim(consoleDump), "");
savecontent variable="browserDump" {
    writeDump(var="rcf207-browser-marker");
}
assertTrue("default output still emits to the page", findNoCase("rcf207-browser-marker", browserDump) GT 0);

// Query dump shows columns, rows, and (for executed queries) timing + SQL.
qn = queryNew("id,name", "integer,varchar");
queryAddRow(qn); querySetCell(qn, "id", 1); querySetCell(qn, "name", "Ada");
r = queryExecute("SELECT id, name FROM qn ORDER BY id", [], { dbtype: "query" });
savecontent variable="qdump" {
    writeDump(r);
}
assertTrue("query dump labelled Query", findNoCase("Query", qdump) GT 0);
assertTrue("query dump shows column", findNoCase("name", qdump) GT 0);
// RustCFML's dump wording; Lucee's markup words the count differently.
if ( isRustCFML() ) {
	assertTrue("query dump shows record count", findNoCase("1 row", qdump) GT 0 OR findNoCase("Records: 1", qdump) GT 0);
}
// Executed (QoQ) queries carry an execution time, surfaced as "ms".
assertTrue("query dump shows execution time", findNoCase(" ms", qdump) GT 0);
// And the originating SQL.
assertTrue("query dump shows SQL", findNoCase("SELECT id, name", qdump) GT 0);

suiteEnd();
</cfscript>
