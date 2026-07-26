<cfsilent>
	<!--- Probe tag: exercises every caller-write shape whose semantics we must
	      match against Lucee (the reference engine) before changing RustCFML's
	      custom-tag caller machinery to a live scope handle. --->
	<cfif thistag.executionmode EQ "start">
		<!--- read of a (possibly shadowed) key BEFORE writing it --->
		<cfset caller.tagReadXBeforeWrite = StructKeyExists( caller, "x" ) ? caller.x : "(absent)" />
		<cfset caller.x = "written-by-tag" />
		<cfset caller.newk = "new-key-from-tag" />
		<cfset StructDelete( caller, "togo" ) />
		<cfif StructKeyExists( caller, "arr" )>
			<cfset ArrayAppend( caller.arr, "appended-by-tag" ) />
		</cfif>
		<!--- record what the tag SEES when reading through caller --->
		<cfset caller.tagSawX = StructKeyExists( caller, "xReadProbe" ) ? caller.xReadProbe : "(absent)" />
	</cfif>
</cfsilent>
