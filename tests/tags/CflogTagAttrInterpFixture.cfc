<!---
    Gap fixture: a <cflog> tag whose quoted `text` attribute combines literal
    text with one or more #expr# interpolations (leading literal, an interior
    segment, a function-call segment, and a trailing literal). This is the exact
    shape Moopa's code/moopa/internal/routing/access_policy.cfc uses:

        <cflog type="information" file="security_check"
               text="Security check for route #...# and endpoint #...# took #getTickCount() - start_time#ms">

    On Lucee 5/6/7, Adobe CF, and BoxLang the engine evaluates each #...# segment
    and preserves the literal text before, between, and after them. On RustCFML
    the cflog tag preprocessor still routes the attribute through strip_hashes(),
    which only handles a value that is ENTIRELY one #expr#; a mixed value emits
    malformed script and the component fails to PARSE. The parse failure surfaces
    at createObject() time as "Could not find the component", which is why this
    construct lives in a runtime-instantiated fixture rather than inline (an
    inline parse error escapes try/catch and would abort the whole runner).
--->
<cfcomponent output="false">
    <cffunction name="emit" returntype="string" output="false">
        <cfset var who = "world" />
        <cflog file="rustcfml_attr_interp_probe" text="hello #who# at #ucase(who)# end" type="information" />
        <cfreturn "ok" />
    </cffunction>
</cfcomponent>
