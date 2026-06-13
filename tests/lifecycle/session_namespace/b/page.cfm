<cfif (url.op ?: form.op ?: "") eq "write">
    <cfset session.x = url.val ?: form.val ?: "" />
</cfif>
<cfoutput>app=[#getApplicationMetadata().name ?: ''#];started=[#session.started_in ?: ''#];x=[#session.x ?: ''#]</cfoutput>
