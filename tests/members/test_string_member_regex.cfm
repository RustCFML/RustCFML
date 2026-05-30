<cfscript>
suiteBegin("String member regex functions");

route = "/sysadmin/routes/[route_id]";
assert("string.reFind passes pattern before receiver", route.reFind("\[\w+\]"), 18);
assert("string.reMatch passes pattern before receiver", arrayToList(route.reMatch("\[\w+\]")), "[route_id]");

suiteEnd();
</cfscript>
