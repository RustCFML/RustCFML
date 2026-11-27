<cfscript>
// GH ##545: cfinvokeargument name must evaluate interpolated expressions
// (including nested double quotes), just like value. TestBox MixerUtil
// invokerMixin relies on listFirst/listLast of key="a=1" pairs.
suiteBegin( "Tags: cfinvokeargument name expression (GH ##545)" );

function f(){ return arguments; }
key = "a=1";
</cfscript>

<!--- 1. Nested double quotes inside name expression (the TestBox / parse-error case) --->
<cfinvoke method="f" returnvariable="r1">
	<cfinvokeargument name="#listFirst( key, "=" )#" value="1">
</cfinvoke>

<!--- 2. Single-quoted delimiter inside the name expression --->
<cfinvoke method="f" returnvariable="r2">
	<cfinvokeargument name="#listFirst( key, '=' )#" value="1">
</cfinvoke>

<!--- 3. Whole-value variable interpolation for name --->
<cfset n = listFirst( key, "=" )>
<cfinvoke method="f" returnvariable="r3">
	<cfinvokeargument name="#n#" value="1">
</cfinvoke>

<!--- 4. Prefix + interpolation --->
<cfset n = "b">
<cfinvoke method="f" returnvariable="r4">
	<cfinvokeargument name="x#n#" value="1">
</cfinvoke>

<!--- 5. Static name, and value with nested double quotes (must keep working) --->
<cfinvoke method="f" returnvariable="r5a">
	<cfinvokeargument name="a" value="1">
</cfinvoke>
<cfinvoke method="f" returnvariable="r5b">
	<cfinvokeargument name="b" value="#listLast( key, "=" )#">
</cfinvoke>

<cfscript>
assert( "name with nested double quotes evaluates", serializeJSON( r1 ), '{"a":"1"}' );
assert( "name with single-quoted delimiter evaluates", serializeJSON( r2 ), '{"a":"1"}' );
assert( "name hash-variable evaluates", serializeJSON( r3 ), '{"a":"1"}' );
assert( "name prefix-plus-interpolation evaluates", serializeJSON( r4 ), '{"xb":"1"}' );
assert( "static name still works", serializeJSON( r5a ), '{"a":"1"}' );
assert( "value with nested double quotes still works", serializeJSON( r5b ), '{"b":"1"}' );

suiteEnd();
</cfscript>
