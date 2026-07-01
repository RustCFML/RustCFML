<cfcomponent>
    <!--- ColdBox 5.4.0 ships `colddoc:generic` on <cfargument> in production
          DSL/interface code (e.g. coldbox.system.ioc.dsl.IDSLBuilder). Lucee
          accepts a namespaced tag attribute on <cfargument>, not just
          <cffunction>. GitHub #226. --->
    <cffunction name="process" returntype="numeric">
        <cfargument name="input" type="string" colddoc:generic="my.package.Widget" inject="coldbox">
        <cfreturn 42>
    </cffunction>
</cfcomponent>
