<!---
    Fixture for test_cfqueryparam_shared_instance_threads.cfm.

    One instance of this component is shared by several threads at once (the
    application-scoped singleton shape: a framework's db/record-writer object
    held in application.lib.db). Each method runs a tag-mode cfquery whose
    placeholder set is fixed, then returns the bound values so the caller can
    check the ROW as well as the absence of an error.
--->
<cfcomponent output="false">

    <!--- The datasource is injected (an inline sqlite JDBC struct in the test)
          so the same fixture runs on Lucee, which has no named `testds`. --->
    <cffunction name="init" access="public" returntype="any" output="false">
        <cfargument name="ds" type="any" required="true" />
        <cfset variables.ds = arguments.ds />
        <cfreturn this />
    </cffunction>

    <!--- Builds a cfqueryparam attribute struct. Called from INSIDE the query
          body below, mid-way through accumulating the statement's parameters
          (the moopa write_mapper.buildQueryParam shape). --->
    <cffunction name="paramFor" access="public" returntype="struct" output="false">
        <cfargument name="v" type="string" required="true" />
        <cfreturn { cfsqltype: "varchar", value: arguments.v, null: false } />
    </cffunction>

    <!--- LEG A shape: the query body calls a method on this same instance while
          its cfqueryparams are being collected. Three placeholders:
          ?  ,  ?  ,  ?   ->  a | b1 | b2 --->
    <cffunction name="selectViaHelper" access="public" returntype="string" output="false">
        <cfargument name="tag" type="string" required="true" />
        <cfargument name="i" type="numeric" required="true" />
        <cfquery name="local.q" datasource="#variables.ds#">
            SELECT <cfqueryparam cfsqltype="integer" value="#arguments.i#" /> AS a
            <cfloop from="1" to="2" index="local.k">
                <cfset local.p = paramFor(arguments.tag) />
                , <cfqueryparam attributeCollection="#local.p#" /> AS b#local.k#
            </cfloop>
        </cfquery>
        <cfreturn local.q.a & "|" & local.q.b1 & "|" & local.q.b2 />
    </cffunction>

    <!--- CONTROL shape: same instance, same three placeholders, but every
          cfqueryparam is inline — no method call inside the query body. --->
    <cffunction name="selectInline" access="public" returntype="string" output="false">
        <cfargument name="tag" type="string" required="true" />
        <cfargument name="i" type="numeric" required="true" />
        <cfquery name="local.q" datasource="#variables.ds#">
            SELECT <cfqueryparam cfsqltype="integer" value="#arguments.i#" /> AS a
                 , <cfqueryparam cfsqltype="varchar" value="#arguments.tag#" /> AS b1
                 , <cfqueryparam cfsqltype="varchar" value="#arguments.tag#" /> AS b2
        </cfquery>
        <cfreturn local.q.a & "|" & local.q.b1 & "|" & local.q.b2 />
    </cffunction>

</cfcomponent>
