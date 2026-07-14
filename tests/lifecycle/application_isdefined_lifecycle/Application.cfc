<cfcomponent output="false">
    <cfset this.name = "isdefLifecycleTest">
    <cfset this.sessionmanagement = false>

    <!--- Mura/Masa declare their lifecycle handlers by INCLUDING a *_method.cfm
          rather than writing `function onApplicationStart(){}` inline. These
          must (a) fire as lifecycle handlers, and (b) be visible to
          isDefined("onApplicationStart") from within another lifecycle method,
          exactly as they are on Lucee. --->
    <cfinclude template="onApplicationStart_method.cfm">
    <cfinclude template="onRequestStart_method.cfm">
</cfcomponent>
