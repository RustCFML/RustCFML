<!---
  Base-tag ancestry outer host. In its End mode it invokes basetag_probe from
  THIS template — that is how real tag libraries nest (a modal tag rendering
  its fragment/slot children). Tags placed in this host's BODY also see it as
  an ancestor when it is cf_-invoked (pinned separately in the test; a
  cfmodule-invoked host would not appear there).

  After the probe runs, reports (via request.btouter) whether the slot the
  probe deposited into attributes through the getBaseTagData() reference is
  visible — the mutation contract the fragment/slot pattern depends on.
--->
<cfif thisTag.executionMode eq "start">
    <cfparam name="attributes.marker" default="outer-marker" />
</cfif>

<cfif thisTag.executionMode eq "end">

    <cf_basetag_probe marker="inner-1" deposit="actions" report="btnested" />

    <cfset request.btouter = {} />
    <cfif structKeyExists(attributes, "slots")>
        <cfset request.btouter.slots_report = structKeyList(attributes.slots) & "=" & (attributes.slots["actions"] ?: "(missing)") />
    <cfelse>
        <cfset request.btouter.slots_report = "(no slots key)" />
    </cfif>
    <cfset thisTag.generatedContent = "" />

</cfif>
