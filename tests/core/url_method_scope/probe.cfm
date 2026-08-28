<cfsetting enablecfoutputonly="true">
<cfcontent type="text/plain; charset=utf-8">
<!---
    One request per shape (?shape=...), so the url scope observed here is this
    request's own and nothing leaks into the runner's request.
    Each line is name=[value]; the test reads them back by name.
--->
<cffunction name="makeAndRead" output="false">
    <cfargument name="path" type="string" required="true" />
    <cfset var o = createObject("component", arguments.path) />
    <cfreturn structKeyList(url) />
</cffunction>

<cfset shape = url.shape ?: "" />
<cfoutput>before=[#structKeyList(url)#];</cfoutput>

<cfif shape EQ "method_url">
    <cfset o = createObject("component", "UrlMethodFixture") />
    <cfoutput>after_create=[#structKeyList(url)#];probe=[#url.probe ?: 'UNDEF'#];call=[#o.url("k")#];</cfoutput>
    <cfset url.written = "yes" />
    <cfoutput>written=[#url.written ?: 'LOST'#];</cfoutput>

<cfelseif shape EQ "method_url_in_function">
    <cfoutput>inside=[#makeAndRead("UrlMethodFixture")#];after=[#structKeyList(url)#];probe=[#url.probe ?: 'UNDEF'#];</cfoutput>

<cfelseif shape EQ "arg_url">
    <cfset o = createObject("component", "UrlArgFixture") />
    <cfoutput>after_create=[#structKeyList(url)#];call=[#o.go(url="v")#];probe=[#url.probe ?: 'UNDEF'#];</cfoutput>

<cfelseif shape EQ "method_form">
    <cfset o = createObject("component", "FormMethodFixture") />
    <cfoutput>after_create=[#structKeyList(url)#];probe=[#url.probe ?: 'UNDEF'#];call=[#o.form()#];</cfoutput>
</cfif>
