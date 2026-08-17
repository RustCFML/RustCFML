<cfsilent>
<!--- Probe for test_cfthread_module_attributes.cfm: reports the `attributes`
      scope the MODULE template sees. Invoked via `module template=…` from
      inside a cfthread that carries its own attributes — the module's own
      attributes must win over the thread's (GH #324). --->
<cfset attrKeys = listSort( structKeyList( attributes ), "textnocase" )>
<cfset marker = structKeyExists( attributes, "marker" ) ? attributes.marker : "(missing)">
</cfsilent><cfoutput>keys=[#attrKeys#] marker=[#marker#]</cfoutput>
