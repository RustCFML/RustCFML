<cfparam name="url.test" default="">

<cfif url.test EQ "default">
    <cfcookie name="ck_default" value="v" samesite="Lax">
    <cfoutput>default-ok</cfoutput>
<cfelseif url.test EQ "explicit">
    <cfcookie name="ck_explicit" value="v" path="/custom" samesite="Strict" httponly="true" secure="true">
    <cfoutput>explicit-ok</cfoutput>
<cfelseif url.test EQ "omitted">
    <cfcookie name="ck_omitted" value="v">
    <cfoutput>omitted-ok</cfoutput>
<cfelse>
    <cfoutput>unknown-test</cfoutput>
</cfif>
