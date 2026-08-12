<!---
  Nestable host for instance-number measurement. When outer (inner="false",
  the default) its End mode invokes a second instance of ITSELF with a
  different marker; the innermost invokes the instance probe from its own
  template, giving the probe an ancestry containing CF_BASETAG_NEST twice:

      CF_BASETAG_INSTANCE_PROBE, CF_BASETAG_NEST (marker=nest-inner),
      CF_BASETAG_NEST (marker=nest-outer), CF_RUNTEST, ...

  Which marker "instance 1" and "instance 2" return is the ordering under
  measurement (nearest-first vs outermost-first).
--->
<cfif thisTag.executionMode eq "start">
    <cfparam name="attributes.marker" default="nest-outer" />
    <cfparam name="attributes.inner" default="false" />
</cfif>

<cfif thisTag.executionMode eq "end">
    <cfif attributes.inner>
        <cf_basetag_instance_probe target="CF_BASETAG_NEST" report="btinst_dup" />
    <cfelse>
        <cf_basetag_nest marker="nest-inner" inner="true"></cf_basetag_nest>
    </cfif>
    <cfset thisTag.generatedContent = "" />
</cfif>
