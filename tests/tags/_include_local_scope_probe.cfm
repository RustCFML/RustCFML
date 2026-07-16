<!--- Included at page (non-function) level: the caller has no function-local
      scope, so page `variables` must NOT appear as `local.*` here. --->
<cfset request._incLocalPageLeak = structKeyExists(local, "_incLocalPageVar")>
