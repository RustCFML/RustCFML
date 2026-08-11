<!---
  Base-tag ancestry probe (leaf). Reports what the engine's custom-tag
  ancestry built-ins expose, via request[attributes.report] (request crosses
  tag boundaries, so the report is readable however deep the probe is nested,
  and a named report key lets several probe runs coexist in one test).

  Attributes:
    report  — request key to write the report struct to (default btprobe)
    marker  — identifies this instance in parent-vs-self disambiguation
    deposit — a slot name to write into the nearest ancestor's attributes
              through the getBaseTagData() reference (the fragment/slot
              pattern real custom-tag libraries use)
--->
<cfif thisTag.executionMode eq "start">

    <cfparam name="attributes.marker" default="(unset)" />
    <cfparam name="attributes.report" default="btprobe" />

    <cfset p = {} />
    <cfset p.list = getBaseTagList() />
    <cfset p.len = listLen(p.list) />
    <cfset p.first = listFirst(p.list) />

    <cfif p.len GTE 2>
        <cfset p.parent_name = listGetAt(p.list, 2) />
        <cftry>
            <cfset p.parent_marker = getBaseTagData(p.parent_name).attributes.marker ?: "(no-marker)" />
            <cfcatch type="any"><cfset p.parent_marker = "(threw: #cfcatch.message#)" /></cfcatch>
        </cftry>
    </cfif>

    <!--- Self lookup by own tag name: the nearest instance is this probe. --->
    <cftry>
        <cfset p.self_marker = getBaseTagData("CF_BASETAG_PROBE").attributes.marker ?: "(no-marker)" />
        <cfcatch type="any"><cfset p.self_marker = "(threw: #cfcatch.message#)" /></cfcatch>
    </cftry>

    <!--- Module-name lookup: even when a CFMODULE entry is in getBaseTagList(),
          getBaseTagData("CFMODULE") cannot find it (Lucee-measured). --->
    <cftry>
        <cfset p.cfmodule_lookup = "found" />
        <cfset dummy = getBaseTagData("CFMODULE") />
        <cfcatch type="any"><cfset p.cfmodule_lookup = "(threw: #cfcatch.message#)" /></cfcatch>
    </cftry>

    <!--- The deposit shape: mutate the ancestor's attributes through the
          returned reference; the ancestor checks for it after the probe
          returns. --->
    <cfif structKeyExists(attributes, "deposit") AND p.len GTE 2>
        <cftry>
            <cfset depositData = getBaseTagData(p.parent_name) />
            <cfif NOT structKeyExists(depositData.attributes, "slots")>
                <cfset depositData.attributes.slots = {} />
            </cfif>
            <cfset depositData.attributes.slots[attributes.deposit] = "deposited-by-probe" />
            <cfcatch type="any"><cfset p.deposit_err = cfcatch.message /></cfcatch>
        </cftry>
    </cfif>

    <!--- Under the suite runner the ancestry always contains CF_RUNTEST exactly
          once, so a by-name lookup is unambiguous: its attributes.file is the
          running test's own registration path. --->
    <cfif listFindNoCase(p.list, "CF_RUNTEST")>
        <cftry>
            <cfset p.runtest_file = getBaseTagData("CF_RUNTEST").attributes.file ?: "(missing)" />
            <cfcatch type="any"><cfset p.runtest_file = "(threw: #cfcatch.message#)" /></cfcatch>
        </cftry>
    <cfelse>
        <cfset p.runtest_file = "(no CF_RUNTEST ancestor)" />
    </cfif>

    <cfset request[attributes.report] = p />

</cfif>
