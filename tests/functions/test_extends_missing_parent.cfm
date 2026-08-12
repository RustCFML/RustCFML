<cfscript>
suiteBegin( "extends a missing component" );

/*
 * A named `extends` target that cannot be found is an ERROR, not a silently
 * parentless component. Verified against Lucee 7.0.4.34:
 *     expression :: invalid component definition, can't find component [Name]
 *
 * Before v0.594.0 RustCFML created the component happily, minus its inherited
 * methods, while `getMetadata( o ).extends` still reported the missing parent —
 * so a typo'd or moved parent degraded silently. (That is exactly how a broken
 * test once impersonated an engine inheritance bug.)
 */

missing = false;
errType = "";
errMsg  = "";
try {
    o = createObject( "component", "orphan_missing_parent" );
    o.own();
} catch ( any e ) {
    missing = true;
    errType = e.type    ?: "";
    errMsg  = e.message ?: "";
}

assertTrue( "extending a non-existent component throws", missing );
assert( "the exception type matches Lucee", errType, "expression" );
assertTrue( "the message names the missing component, Lucee's wording",
            errMsg contains "can't find component" && errMsg contains "NoSuchParentComponent_rcfml" );

// An EXISTING parent still resolves normally — the guard must not over-fire.
ok = createObject( "component", "orphan_real_parent" );
assert( "a real parent still inherits", ok.inherited(), "FROM-PARENT" );
assert( "and the child's own method still works", ok.own(), "OWN" );

suiteEnd();
</cfscript>
