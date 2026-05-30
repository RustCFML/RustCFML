<cfscript>
suiteBegin("CFLoop collection item and index");

tables = {
    route = {
        fields = {
            profiles = {}
        }
    }
};
</cfscript>
<cfloop collection="#tables#" item="table" index="tableName">
    <cfset table.visited = tableName />
</cfloop>
<cfscript>
assert("collection cfloop exposes item value and key index", tables.route.visited ?: "missing", "route");

schema = {
    route = {
        fields = {
            profiles = {}
        }
    }
};
</cfscript>
<cfloop collection="#schema.route.fields#" item="field" index="fieldName">
    <cfset field.generated = fieldName />
</cfloop>
<cfscript>
assert("collection cfloop item mutation writes back", schema.route.fields.profiles.generated ?: "missing", "profiles");

suiteEnd();
</cfscript>
