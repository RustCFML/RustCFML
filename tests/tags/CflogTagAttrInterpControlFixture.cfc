<!---
    Control fixture: a <cflog> tag whose quoted `text` attribute is a SINGLE
    bare interpolation (#expr#) with no surrounding literal text. RustCFML
    already parses this shape, so it is the regression guard that proves the
    fixture wiring (createObject + emit) is sound. A failure on the sibling
    CflogTagAttrInterpFixture therefore isolates to the literal-text-mixed-with-
    interpolation case, not to cflog tag handling in general.
--->
<cfcomponent output="false">
    <cffunction name="emit" returntype="string" output="false">
        <cfset var who = "world" />
        <cflog file="rustcfml_attr_interp_probe" text="#who#" type="information" />
        <cfreturn "ok" />
    </cffunction>
</cfcomponent>
