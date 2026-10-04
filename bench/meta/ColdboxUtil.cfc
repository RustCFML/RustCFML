<cfcomponent output="false">
<!--- getInheritedMetaData --->
	<cffunction name="getInheritedMetaData" output="false" hint="Returns a single-level metadata struct that includes all items inhereited from extending classes.">
		<cfargument name="component" type="any" required="true" hint="A component instance, or the path to one">
		<cfargument name="stopRecursions" default="#arraynew(1)#" hint="An array of classes to stop recursion">
		<cfargument name="md" default="#structNew()#" hint="A structure containing a copy of the metadata for this level of recursion.">

		<cfset var loc = {}>

		<!--- First time through, get metaData of component.  --->
		<cfif structIsEmpty( md )>
			<cfif isObject( component )>
				<cfset md = getMetaData( component )>
			<cfelse>
				<cfset md = getComponentMetaData( component )>
			</cfif>
		</cfif>

		<!--- If it has a parent, stop and calculate it first, unless of course, we've reached a class we shouldn't recurse into. --->

		<cfif 	structKeyExists( md, "extends" ) AND
				md.type eq "component" AND
				stopClassRecursion( md.extends.name, arguments.stopRecursions ) EQ FALSE
		>
			<cfset loc.parent = getInheritedMetaData( component=component, stopRecursions=stopRecursions, md=md.extends )>
		<!--- If we're at the end of the line, it's time to start working backwards so start with an empty struct to hold our condensesd metadata. --->
		<cfelse>
			<cfset loc.parent = {}>
			<cfset loc.parent.inheritancetrail = []>
		</cfif>

		<!--- Override ourselves into parent --->
		<cfloop collection="#md#" item="loc.key">
			<!--- Functions and properties are an array of structs keyed on name, so I can treat them the same --->
			<cfif listFindNoCase( "functions,properties", loc.key )>
				<cfif not structKeyExists( loc.parent, loc.key )>
					<cfset loc.parent[ loc.key ] = []>
				</cfif>
				<!--- For each function/property in me... --->
				<cfloop array="#md[ loc.key ]#" index="loc.item">
					<cfset loc.parentItemCounter = 0>
					<cfset loc.foundInParent = false>
					<!--- ...Look for an item of the same name in my parent... --->
					<cfloop array="#loc.parent[ loc.key ]#" index="loc.parentItem">
						<cfset loc.parentItemCounter++>
						<!--- ...And override it --->
						<cfif compareNoCase( loc.item.name, loc.parentItem.name ) eq 0>
							<cfset loc.parent[ loc.key ][ loc.parentItemCounter ] = loc.item>
							<cfset loc.foundInParent = true>
							<cfbreak>
						</cfif>
					</cfloop>
					<!--- ...Or add it --->
					<cfif not loc.foundInParent>
						<cfset arrayAppend( loc.parent[ loc.key ], loc.item )>
					</cfif>
				</cfloop>
			<cfelseif NOT listFindNoCase( "extends,implements", loc.key )>
				<cfset loc.parent[ loc.key ] = md[ loc.key ]>
			</cfif>
		</cfloop>
		<cfset arrayPrePend( loc.parent.inheritanceTrail, loc.parent.name )>
		<cfreturn loc.parent>
	</cffunction>

	<cffunction name="stopClassRecursion" access="private" returntype="any" hint="Should we stop recursion or not due to class name found: Boolean" output="false" doc_generic="Boolean">
		<cfargument name="classname" 	required="true" hint="The class name to check">
		<cfargument name="stopRecursions"	required="true" hint="An array of classes to stop processing at"/>
		<cfscript>
			// Try to find a match
			for( var thisClass in arguments.stopRecursions ){
				if( compareNoCase( thisClass, arguments.classname) eq 0 ){
					return true;
				}
			}
			return false;
		</cfscript>
	</cffunction>
</cfcomponent>
