<!--- Reads the caller's function-`local` scope. A <cfinclude> runs in the same
      function-local scope as its caller, so `local.category` set before the
      include is visible here (Masa CMS cplugins/dsp_table.cfm regression). --->
<cfset request._incLocalCategory = local.category>
<cfset request._incLocalStructExists = structKeyExists(local, "category")>
