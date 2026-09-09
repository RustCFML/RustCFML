component extends="Mid" {
	request.ctorsemLog = listAppend( request.ctorsemLog, "Leaf" );
	function init() { return this; }
	function leafMethod() { return "leaf-method"; }
	function seenFromRoot() { return variables.fromRoot; }
	function viaSuper() { return super.midMethod() & "+" & super.rootMethod(); }
	function ownKeys() { return structKeyList( this ); }
}
