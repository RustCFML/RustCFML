/**
 * GH ##420 — `private` reaches DOWN the inheritance chain (the `variables` scope
 * is shared with the subclass), and a closure minted inside a method carries the
 * component it was minted in, so it reads as an insider too.
 */
component extends="MemberViewFixture" {

	public string function readInheritedPrivate() {
		return "bracket=" & ( isNull( this[ "priv" ] )   ? "NULL" : "fn" )
		     & " data="    & ( isNull( this[ "secret" ] ) ? "NULL" : this[ "secret" ] );
	}

	public string function readFromClosure() {
		var f = function() { return isNull( this[ "priv" ] ) ? "NULL" : "fn"; };
		return f();
	}

}
