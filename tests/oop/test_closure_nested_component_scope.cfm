<cfscript>
suiteBegin( "A closure called from inside another closure keeps its component's scopes" );
r = new oop.clobind.Recur();
assert( "recursive var-scoped closure reaches a private method at every depth", arrayToList( r.recurse(), "|" ), "1:priv:recur:recur-this|2:priv:recur:recur-this|3:priv:recur:recur-this" );
assert( "closure calling a sibling closure reaches a private method", r.sibling(), "priv/recur" );
t = new oop.clobind.Tgt();
t.inject( "walk", r.mkRecursive() );
assert( "recursive closure invoked on another component keeps its definer's scopes", arrayToList( t.walk( 1 ), "|" ), "priv:recur|priv:recur" );
suiteEnd();
</cfscript>
