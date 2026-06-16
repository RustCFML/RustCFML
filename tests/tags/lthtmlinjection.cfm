<cfif thisTag.executionMode EQ "start">
    <cfhtmlhead text='<meta name="x-rustcfml-head" content="head-value">'>
    <cfhtmlbody text='<script id="x-rustcfml-body">window.__rustcfmlBody = true;</script>'>
    <cfset caller[attributes.outVar] = "ran">
</cfif>
