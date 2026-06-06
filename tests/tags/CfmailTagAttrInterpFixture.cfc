<!---
    Gap fixture: a <cfmail> tag whose quoted attributes combine literal text with
    #expr# interpolations (the subject mixes a leading literal, an interior
    segment, a function-call segment, and a trailing literal). Same root cause as
    the cflog gap: the cfmail tag preprocessor routes attributes through
    strip_hashes() instead of format_attr_value(), so a mixed value emits
    malformed script and the component fails to PARSE.

    The <cfmail> is guarded by <cfif false> so it is compiled but never executed
    (cfmail with no SMTP server throws at runtime on every engine; this test
    isolates PARSE-time behavior). The parse failure surfaces at createObject()
    time as "Could not find the component", hence the fixture rather than inline.
--->
<cfcomponent output="false">
    <cffunction name="emit" returntype="string" output="false">
        <cfset var who = "world" />
        <cfif false>
            <cfmail to="a@b.c" from="c@d.e" subject="hello #who# at #ucase(who)# end">body</cfmail>
        </cfif>
        <cfreturn "ok" />
    </cffunction>
</cfcomponent>
