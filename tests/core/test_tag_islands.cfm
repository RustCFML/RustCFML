<cfscript>
suiteBegin("Tag islands in script");

// Plain markup, nested tags, interpolation, capture and script resumption.
function tagIslandCapture() {
    var text = "";
    var total = 0;
    ```
    <cfsavecontent variable="local.text"><cfloop from="1" to="3" index="local.i"><cfif local.i GT 1><cfoutput><b>#local.i#</b></cfoutput><cfset local.total += local.i></cfif></cfloop></cfsavecontent>
    ```
    return { text = text, total = total };
}
result = tagIslandCapture();
assert("nested markup and interpolation", result.text, "<b>2</b><b>3</b>");
assert("tag assignments share function locals", result.total, 5);

function tagIslandQuery() {
    var sourceQuery = queryNew("id", "integer", [[1],[2]]);
    var filtered = "";
    ```
    <cfquery name="local.filtered" dbtype="query">
        SELECT id FROM sourceQuery WHERE id = <cfqueryparam value="2" cfsqltype="integer">
    </cfquery>
    ```
    return filtered;
}
filtered = tagIslandQuery();
assert("queryparam inside query island", filtered.recordCount, 1);
assert("query result value", filtered.id[1], 2);

function tagIslandReturn() {
    ```<cfreturn "returned">```
    return "wrong";
}
assert("tag return exits enclosing function", tagIslandReturn(), "returned");

function tagIslandBreak() {
    var count = 0;
    for (var i = 1; i <= 4; i++) {
        ```<cfif i EQ 3><cfbreak></cfif><cfset local.count += 1>```
    }
    return count;
}
assert("tag break exits enclosing script loop", tagIslandBreak(), 2);

function tagIslandNestedScript() {
    var result = "";
    ```<cfscript>
        local.result = "before";
        ```<cfset local.result &= "-inside">```
    </cfscript>```
    return result & "-after";
}
assert("nested script and island", tagIslandNestedScript(), "before-inside-after");

function tagIslandTry() {
    var caught = "";
    ```<cftry><cfthrow message="island-error"><cfcatch type="any"><cfset local.caught = cfcatch.message></cfcatch></cftry>```
    return caught;
}
assert("tag try/catch", tagIslandTry(), "island-error");

function tagIslandCustomTag() {
    var text = "";
    ```<cfimport taglib="fixtures" prefix="island">```
    ```<cfsavecontent variable="local.text"><island:island_custom value="custom" /></cfsavecontent>```
    return trim(text);
}
assert("imported custom tag across islands", tagIslandCustomTag(), "custom");

function tagIslandInclude() {
    var value = 40;
    ```<cfinclude template="fixtures/island_include.cfm">```
    return value;
}
assert("included template shares locals", tagIslandInclude(), 42);

probe = new core.TagIslandProbe();
assert("CFC method islands execute on call", probe.run(), 42);
assert("CFC construction did not execute method body", probe.ranDuringConstruction, false);
suiteEnd();
</cfscript>
