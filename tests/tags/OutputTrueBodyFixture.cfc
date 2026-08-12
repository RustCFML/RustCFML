<cfcomponent hint="Fixture: the three output modes of a tag-based cffunction body.">

    <!--- The gap under test: output='true' means the body is processed AS IF
          INSIDE CFOUTPUT — hash expressions interpolate and ## collapses to a
          literal # — with no explicit cfoutput anywhere. --->
    <cffunction name="implicitBody" output="true">
        <cfargument name="val" default="42" />
        IMPLICIT VAL:#arguments.val# ESC:[##] TICK:[`Job ##${js}`] END
    </cffunction>

    <!--- Control: the same body explicitly wrapped — works on both engines. --->
    <cffunction name="explicitBody" output="true">
        <cfargument name="val" default="42" />
        <cfoutput>EXPLICIT VAL:#arguments.val# ESC:[##] END</cfoutput>
    </cffunction>

    <!--- Control: output='false' suppresses body text entirely; only the
          return value escapes. --->
    <cffunction name="suppressedBody" output="false">
        <cfargument name="val" default="42" />
        SUPPRESSED VAL:#arguments.val# SHOULD-NOT-APPEAR
        <cfreturn "ret:#arguments.val#" />
    </cffunction>

    <!--- Measured leg: no output attribute at all. --->
    <cffunction name="defaultBody">
        <cfargument name="val" default="42" />
        DEFAULT VAL:#arguments.val# ESC:[##] END
    </cffunction>

</cfcomponent>
