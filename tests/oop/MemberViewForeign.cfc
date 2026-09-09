/**
 * GH ##420 — a component of a DIFFERENT class is an outsider: being inside SOME
 * component is not the same as being inside THIS one. Pairs with
 * `MemberViewFixture`; see `test_component_member_view_by_reader.cfm`.
 */
component {

	variables.mySecret = "foreign-secret";

	public string function probeOther( required any other ) {
		return "priv="       & ( isNull( other[ "priv" ] )   ? "NULL" : "fn" )
		     & " bracket="   & ( isNull( other[ "secret" ] ) ? "NULL" : other[ "secret" ] )
		     & " dot="       & ( isNull( other.secret )      ? "NULL" : other.secret );
	}

}
