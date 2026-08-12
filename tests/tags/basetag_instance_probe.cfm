<!---
  getBaseTagData() instance-number probe. For a target ancestor tag name,
  records what each spelling of the instance argument returns — the marker
  attribute of whichever instance the engine resolved, or the thrown message.
  Reports via request[attributes.report].

  Attributes:
    target — ancestor tag name to look up (e.g. CF_BASETAG_NEST)
    report — request key for the report struct (default btinst)
--->
<cfif thisTag.executionMode eq "start">

    <cfparam name="attributes.target" type="string" />
    <cfparam name="attributes.report" default="btinst" />

    <cfset p = {} />
    <cfset p.list = getBaseTagList() />

    <!--- Default argument (documented default: 1). --->
    <cftry>
        <cfset p.inst_default = getBaseTagData(attributes.target).attributes.marker ?: "(no-marker)" />
        <cfcatch type="any"><cfset p.inst_default = "(threw: #cfcatch.message#)" /></cfcatch>
    </cftry>

    <!--- Explicit instances 0-3. When a lookup returns a struct without the
          marker (Lucee's out-of-range shape), record what the struct held. --->
    <cfloop from="0" to="3" index="i">
        <cftry>
            <cfset d = getBaseTagData(attributes.target, i) />
            <cfset p["inst_#i#"] = d.attributes.marker ?: "(struct keys: #structKeyList(d)#; attributes keys: #structKeyExists(d, 'attributes') ? structKeyList(d.attributes) : 'NO ATTRIBUTES KEY'#)" />
            <cfcatch type="any"><cfset p["inst_#i#"] = "(threw: #cfcatch.message#)" /></cfcatch>
        </cftry>
    </cfloop>

    <cfset request[attributes.report] = p />

</cfif>
