<cfscript>
suiteBegin("Soft-keyword function names");

route = createObject("component", "oop.MetadataSoftKeywordRoute");
md = getMetaData(route);

functionNames = [];
for (fn in md.functions) {
    arrayAppend(functionNames, fn.name);
}

assertTrue("component metadata preserves soft-keyword function name", arrayFind(functionNames, "new") > 0);
assertTrue("soft-keyword function is callable", route.new().ok);

suiteEnd();
</cfscript>
