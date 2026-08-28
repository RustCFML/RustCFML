<cfcomponent output="false" hint="A component whose public API includes a method named url() -- e.g. an S3 proxy exposing url(key). Lucee: a method name never touches the caller's scopes.">
    <cffunction name="url" returntype="string" output="false">
        <cfargument name="key" type="string" required="false" default="" />
        <cfreturn "fn-url:" & arguments.key />
    </cffunction>
</cfcomponent>
