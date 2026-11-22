<!--- Fixture for test_cfcatch_type_literal.cfm: throws request.cfcatchThrowType and says which cfcatch took it. --->
<cfset request.cfcatchWhere = "none" />
<cftry>
    <cftry>
        <cfthrow type="#request.cfcatchThrowType#" message="thrown by the fixture" />
        <cfcatch type="app.one,app.two"><cfset request.cfcatchWhere = "typed" /></cfcatch>
    </cftry>
    <cfcatch type="any"><cfset request.cfcatchWhere = "any" /></cfcatch>
</cftry>
