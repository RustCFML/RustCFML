/**
 * GH #351 — probes the `local` name from a pseudo-constructor.
 *
 * A CFC body (the pseudo-constructor) is a template-shaped frame: like a page,
 * Lucee gives it no `local` scope, so `local` there is an ordinary variable that
 * lands in the component's `variables`. Recorded at construction time because
 * the questions cannot be asked again later from a method, where `local` IS a
 * scope.
 */
component {

	variables.isDefinedBefore = isDefined( "local" );

	local.ctorKey = 1;

	// The Wheels shape: a `for ( local.X in local.Y )` loop in a pseudo-constructor.
	// The loop variable is `variables.local.X`, so stripping the prefix (the
	// function-scope normalisation) made the body read an unset variable and the
	// loop do nothing at all — silently.
	local.srcList  = [ 1, 2, 3 ];
	local.loopSeen = "";
	for ( local.entry in local.srcList ) {
		local.loopSeen = listAppend( local.loopSeen, local.entry );
	}
	variables.loopSeen = local.loopSeen;

	variables.createdVariablesLocal = structKeyExists( variables, "local" );
	variables.readBack              = variables.local.ctorKey;
	variables.leakedToVariables     = structKeyExists( variables, "ctorKey" );

	public boolean function getIsDefinedBefore()      { return variables.isDefinedBefore; }
	public boolean function getCreatedVariablesLocal(){ return variables.createdVariablesLocal; }
	public         function getReadBack()             { return variables.readBack; }
	public boolean function getLeakedToVariables()    { return variables.leakedToVariables; }
	public         function getLoopSeen()             { return variables.loopSeen; }

	/**
	 * A method DOES own a `local` scope — it must not see the pseudo-constructor's
	 * `variables.local` struct through it.
	 */
	public boolean function methodHasOwnLocalScope() {
		local.methodKey = "m";
		return structKeyExists( local, "methodKey" ) AND NOT structKeyExists( local, "ctorKey" );
	}

}
