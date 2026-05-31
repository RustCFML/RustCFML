<cfscript>
suiteBegin("For-in keyword member access");

local = {};
items = ["ok"];
for (var local.package in items) {
    keywordLoopResult = local.package;
}

assert("script for-in allows keyword property in dotted variable", keywordLoopResult, "ok");

suiteEnd();
</cfscript>
