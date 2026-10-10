<!---
  Base-tag LOOKUP probe (leaf). Reports what getBaseTagData() resolves for the
  same ancestor asked for under several spellings, and what getBaseTagList()
  does with a delimiter argument.

  getBaseTagData() matches on the TEMPLATE, not on the name getBaseTagList()
  shows: a leading `cf_` is optional and case is irrelevant, so `CF_X`, `X` and
  `x` all find `x.cfm` however it was invoked. `CFMODULE` is not a template
  stem, so it resolves to nothing even when the list shows it.

  Attributes:
    report — request key to write the report struct to
    marker — identifies this instance
--->
<cfif thisTag.executionMode eq "start">

<cfparam name="attributes.marker" default="(unset)" />
<cfparam name="attributes.report" default="btlookup" />

<cfset r = {} />
<cfset r.first = listFirst( getBaseTagList() ) />
<cfset r.semi  = "" />
<cftry>
    <cfset r.semi = getBaseTagList( ";" ) />
    <cfcatch type="any"><cfset r.semi = "(threw: #cfcatch.message#)" /></cfcatch>
</cftry>

<!--- The same ancestor, asked for four ways, plus one that must NOT resolve. --->
<cfset spellings = {
      prefixed = "CF_BASETAG_LOOKUP_PROBE"
    , bare     = "BASETAG_LOOKUP_PROBE"
    , lower    = "basetag_lookup_probe"
    , mixed    = "Cf_BaseTag_Lookup_Probe"
    , module   = "CFMODULE"
} />
<cfset r.lookup = {} />
<cfloop collection="#spellings#" item="spellingKey">
    <cftry>
        <!--- The call is kept OUT of the `?:`. Both engines let the elvis
              operator swallow a thrown exception, so writing this as
              `getBaseTagData( x ).attributes.marker ?: "(no-marker)"` would
              report a NOT-FOUND as a found-but-unmarked tag and quietly pass. --->
        <cfset found = getBaseTagData( spellings[ spellingKey ] ) />
        <cfset r.lookup[ spellingKey ] = found.attributes.marker ?: "(no-marker)" />
        <cfcatch type="any"><cfset r.lookup[ spellingKey ] = "(threw)" /></cfcatch>
    </cftry>
</cfloop>

<!--- The not-found message echoes the requested name AS WRITTEN. --->
<cfset r.missingMsg = "" />
<cftry>
    <cfset getBaseTagData( "CF_NoSuchTag" ) />
    <cfcatch type="any"><cfset r.missingMsg = cfcatch.message /></cfcatch>
</cftry>

<cfset request[ attributes.report ] = r />

</cfif>
