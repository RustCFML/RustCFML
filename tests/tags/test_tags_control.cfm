<cfscript>suiteBegin("Tags: Control Flow");</cfscript>

<!--- cfswitch with string values --->
<cfset color = "red">
<cfswitch expression="#color#">
    <cfcase value="blue">
        <cfset switchResult = "found blue">
    </cfcase>
    <cfcase value="red">
        <cfset switchResult = "found red">
    </cfcase>
    <cfcase value="green">
        <cfset switchResult = "found green">
    </cfcase>
</cfswitch>
<cfscript>assert("cfswitch string match", switchResult, "found red");</cfscript>

<!--- cfswitch with cfdefaultcase --->
<cfset fruit = "mango">
<cfswitch expression="#fruit#">
    <cfcase value="apple">
        <cfset defaultResult = "apple">
    </cfcase>
    <cfcase value="banana">
        <cfset defaultResult = "banana">
    </cfcase>
    <cfdefaultcase>
        <cfset defaultResult = "other">
    </cfdefaultcase>
</cfswitch>
<cfscript>assert("cfswitch defaultcase", defaultResult, "other");</cfscript>

<!--- cfswitch with numeric expression --->
<cfset num = 2>
<cfswitch expression="#num#">
    <cfcase value="1">
        <cfset numSwitchResult = "one">
    </cfcase>
    <cfcase value="2">
        <cfset numSwitchResult = "two">
    </cfcase>
    <cfcase value="3">
        <cfset numSwitchResult = "three">
    </cfcase>
</cfswitch>
<cfscript>assert("cfswitch numeric", numSwitchResult, "two");</cfscript>

<!--- cfbreak in a loop --->
<cfset breakResult = "">
<cfloop index="i" from="1" to="10">
    <cfif i GT 3>
        <cfbreak>
    </cfif>
    <cfset breakResult = breakResult & i>
</cfloop>
<cfscript>assert("cfbreak in loop", breakResult, "123");</cfscript>

<!--- cfcontinue in a loop --->
<cfset contResult = "">
<cfloop index="i" from="1" to="5">
    <cfif i EQ 3>
        <cfcontinue>
    </cfif>
    <cfset contResult = contResult & i>
</cfloop>
<cfscript>assert("cfcontinue in loop", contResult, "1245");</cfscript>

<!--- Nested cfloop --->
<cfset nestedResult = "">
<cfloop index="i" from="1" to="3">
    <cfloop index="j" from="1" to="2">
        <cfset nestedResult = nestedResult & i & j & ",">
    </cfloop>
</cfloop>
<cfscript>assert("nested cfloop", nestedResult, "11,12,21,22,31,32,");</cfscript>

<!--- Nested loop with break (inner only) --->
<cfset nestedBreak = "">
<cfloop index="i" from="1" to="3">
    <cfloop index="j" from="1" to="5">
        <cfif j GT 2>
            <cfbreak>
        </cfif>
        <cfset nestedBreak = nestedBreak & i & j & ",">
    </cfloop>
</cfloop>
<cfscript>assert("nested loop inner break", nestedBreak, "11,12,21,22,31,32,");</cfscript>

<!--- cfif with AND/OR --->
<cfset a = 5>
<cfset b = 10>
<cfif a GT 3 AND b GT 8>
    <cfset logicResult = "both">
<cfelse>
    <cfset logicResult = "nope">
</cfif>
<cfscript>assert("cfif AND logic", logicResult, "both");</cfscript>

<!--- cfif with NOT --->
<cfset isActive = false>
<cfif NOT isActive>
    <cfset notResult = "inactive">
<cfelse>
    <cfset notResult = "active">
</cfif>
<cfscript>assert("cfif NOT", notResult, "inactive");</cfscript>

<!--- Multiple cfelseif chain --->
<cfset score = 75>
<cfif score GTE 90>
    <cfset grade = "A">
<cfelseif score GTE 80>
    <cfset grade = "B">
<cfelseif score GTE 70>
    <cfset grade = "C">
<cfelseif score GTE 60>
    <cfset grade = "D">
<cfelse>
    <cfset grade = "F">
</cfif>
<cfscript>assert("cfelseif chain", grade, "C");</cfscript>

<!--- cfswitch/cfcase bodies nested inside <cfoutput> must interpolate #expr#
      just like <cfif> bodies do. Regression: Preside admin sitetree rendered
      the literal `#translateResource(...)#` in a <cfdefaultcase> because the
      case body lost the enclosing cfoutput context. --->
<cffunction name="_ctrlEcho"><cfargument name="v"><cfreturn "T[" & arguments.v & "]"></cffunction>
<cfset greenClass = "green">
<cfset restriction = "none">
<cfsavecontent variable="switchOut"><cfoutput><cfswitch expression="#restriction#"><cfcase value="full"><i class="#greenClass#"></i>#_ctrlEcho( "full" )#</cfcase><cfdefaultcase><i class="#greenClass#"></i>#_ctrlEcho( "none" )#</cfdefaultcase></cfswitch></cfoutput></cfsavecontent>
<cfscript>assert("cfswitch defaultcase interpolates inside cfoutput", Trim( switchOut ), '<i class="green"></i>T[none]');</cfscript>
<cfset restriction = "full">
<cfsavecontent variable="switchOut2"><cfoutput><cfswitch expression="#restriction#"><cfcase value="full"><i class="#greenClass#"></i>#_ctrlEcho( "full" )#</cfcase><cfdefaultcase>#_ctrlEcho( "none" )#</cfdefaultcase></cfswitch></cfoutput></cfsavecontent>
<cfscript>assert("cfswitch case interpolates inside cfoutput", Trim( switchOut2 ), '<i class="green"></i>T[full]');</cfscript>

<cfscript>suiteEnd();</cfscript>
