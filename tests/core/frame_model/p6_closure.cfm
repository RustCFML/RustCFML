<cfscript>
fmClosure = function() {
	return {
		sawVariables: structKeyExists( variables, "fmCompVar" ),
		sawThis: isDefined( "this" ) && structKeyExists( this, "fmPublic" ),
		sawMethodLocal: isDefined( "fmMethodLocal" ),
		methodLocalValue: isDefined( "fmMethodLocal" ) ? fmMethodLocal : "<undefined>"
	};
};
</cfscript>
