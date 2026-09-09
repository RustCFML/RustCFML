/**
 * GH #420 / #417 — the member VIEW a read through a component receiver gets is
 * decided by the READER, not by the syntax used.
 *
 * Inside the component, `this.priv`, `this[ "priv" ]` and `variables[ "priv" ]`
 * are the same read and Lucee resolves all three. Outside, none of the private
 * scope is reachable at all.
 */
component {

	variables.secret = "shh";

	public  string function pub()  { return "pub"; }
	private string function priv() { return "priv"; }

	/** Every in-component spelling of the same private-member read. */
	public string function nativeLookup() {
		return "dot="     & ( isNull( this.priv )        ? "NULL" : "fn" )
		     & " bracket=" & ( isNull( this[ "priv" ] )   ? "NULL" : "fn" )
		     & " vars="    & ( isNull( variables[ "priv" ] ) ? "NULL" : "fn" );
	}

	/** A private method extracted by bracket and invoked — the mixin shape. */
	public string function callThroughBracket() {
		var f = this[ "priv" ];
		return f();
	}

	/**
	 * Preside's object-merger shape: walk `getMetaData( this ).functions` and
	 * re-home each one by `this[ func.name ]`. Every private method came back
	 * Null on v0.655.0, so the receiving `$addFunction( name, func )` threw
	 * "The parameter [func] ... is required but was not passed in".
	 */
	public numeric function reHomeOwnFunctions( required struct target ) {
		var moved = 0;
		for( var func in getMetaData( this ).functions ) {
			if ( !isNull( this[ func.name ] ) ) {
				target[ func.name ] = this[ func.name ];
				moved++;
			}
		}
		return moved;
	}

	/** `private` is CLASS-level: a sibling instance's private scope is readable. */
	public string function readSibling( required any other ) {
		return ( isNull( other[ "priv" ] ) ? "NULL" : "fn" )
		     & "/" & ( isNull( other[ "secret" ] ) ? "NULL" : other[ "secret" ] );
	}

}
