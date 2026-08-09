<!---
    Gap fixture: a cfparam name= whose target path uses a BRACKET key
    containing a colon -- name="item['x-bind:href']" (an Alpine.js attribute
    name). Lucee parses the name as an lvalue path with a quoted-string key
    and creates the key. RustCFML fails to PARSE ("Expected RBracket, found
    Colon"), degrading this component to a catchable createObject() failure
    -- hence the runtime-instantiated fixture. The same key works in ordinary
    expressions on RustCFML (item['x-bind:href'] = "x" -- asserted in the
    test file), so the gap is specific to the cfparam name path parser.
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfset item = {}>
        <cfparam name="item['x-bind:href']" default="x">
        <cfreturn item['x-bind:href']>
    </cffunction>
</cfcomponent>
