<cffunction name="onRequestStart" output="false" returntype="boolean">
    <cfargument name="targetPage" required="false" default="">
    <!--- Mura/Masa guard: only proceed when the lifecycle method is visible.
          isDefined() must see a sibling lifecycle method that was attached to
          the Application component via include (not an inline declaration). --->
    <cfset request.sawOnAppStart = isDefined("onApplicationStart")>
    <cfset request.sawViaVariables = isDefined("variables.onApplicationStart")>
    <cfreturn true>
</cffunction>
