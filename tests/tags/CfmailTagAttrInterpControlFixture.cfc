<!---
    Control fixture: a <cfmail> tag whose quoted attributes are SINGLE bare
    interpolations (#expr#) with no surrounding literal text. RustCFML already
    parses this shape. The <cfmail> is guarded by <cfif false> so it is compiled
    but never executed -- cfmail with no SMTP server throws at RUNTIME on every
    engine, and this test is about PARSE-time behavior, not delivery. This
    fixture proves the cfmail-in-fixture wiring is sound so a failure on the
    sibling CfmailTagAttrInterpFixture isolates to the mixed literal+#expr# case.
--->
<cfcomponent output="false">
    <cffunction name="emit" returntype="string" output="false">
        <cfset var who = "world" />
        <cfif false>
            <cfmail to="#who#" from="#who#" subject="#who#">body</cfmail>
        </cfif>
        <cfreturn "ok" />
    </cffunction>
</cfcomponent>
