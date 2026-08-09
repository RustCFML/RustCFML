<!---
    Control fixture: a cfparam name= with a DOTTED member path
    (name="item.href"). RustCFML already parses and applies this, so it
    guards the cfparam-in-fixture wiring. (A bracket key WITHOUT a colon,
    name="item['href']", is not usable as the control: RustCFML parses it but
    resolves the quoted key as an identifier at runtime -- "Variable 'href'
    is undefined" -- a neighbouring gap not pinned here.)
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfset item = {}>
        <cfparam name="item.href" default="y">
        <cfreturn item.href>
    </cffunction>
</cfcomponent>
