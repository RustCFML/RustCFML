component extends="Mid" {
	request.ctorsemLog = listAppend( request.ctorsemLog, "Leaf" );
	// Inside a subclass pseudo-constructor the parent chain has already run on
	// this same `this`, so its methods are visible there (Lucee parity).
	variables.ctorInherited = structKeyExists( this, "rootMethod" ) & "/" & structKeyExists( this, "midMethod" )
		& "/" & isDefined( "this.rootMethod" ) & "/" & isNull( this.rootMethod ) & "/" & structKeyExists( this, "leafMethod" );
	variables.ctorThisKeys = listSort( structKeyList( this ), "textnocase" );
	variables.ctorInheritedRef = this.rootMethod;
	function init() { return this; }
	function leafMethod() { return "leaf-method"; }
	function seenFromRoot() { return variables.fromRoot; }
	function viaSuper() { return super.midMethod() & "+" & super.rootMethod(); }
	function ownKeys() { return structKeyList( this ); }
	function inheritedOnThis() { return variables.ctorInherited; }
	function thisKeysInCtor() { return variables.ctorThisKeys; }
	function inheritedRefWorks() { return isCustomFunction( variables.ctorInheritedRef ); }
}
