<cfscript>
// <cfinvoke> with <cfinvokeargument> CHILD tags must pass those children as
// method arguments. Previously the tag preprocessor read only attributes on the
// <cfinvoke> tag itself and dropped every <cfinvokeargument>, so the method ran
// with an empty argument struct. Mura/Masa's plugin event dispatch relies on
// this to pass its scope object as an argument literally named `$`; the missing
// arg surfaced as "Variable '$' is undefined" in the event handler.
suiteBegin( "Tags: cfinvoke with cfinvokeargument children" );

handler = createObject( "component", "CfInvokeArgChildrenFixture" );
scope = { name: "muraScope" };
</cfscript>

<cfinvoke component="#handler#" method="onEvent" returnvariable="result">
	<cfinvokeargument name="event" value="onGlobalRequestStart">
	<cfinvokeargument name="$" value="#scope#">
	<cfinvokeargument name="mura" value="#scope#">
</cfinvoke>

<cfscript>
assert( "cfinvokeargument children passed to method",
    result, "onGlobalRequestStart|muraScope|muraScope" );

// Value passed as a whole-value #expr# keeps its native type (a struct here),
// not a stringified form.
assertTrue( "cfinvokeargument $ arg arrived as a struct", isStruct( scope ) );

suiteEnd();
</cfscript>
