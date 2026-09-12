<cfscript>
suiteBegin( "A var-local assigned the same handle a variables member holds is still created" );
// Regression: a member write compiles to a mutation of the handle plus a
// store of that handle back to its variable. A shortcut that skipped the
// store whenever "the variable already holds this handle" looked the name up
// in the component scope too — so `local.routes = this.getRoutes()` (the same
// array as `variables.routes`) never created the local, and the next
// `local.routes` read threw "Variable 'routes' is undefined" (Wheels
// mapperModernSpec, 9 specs). A `var`-declared name always targets the local.
bag = new oop.localalias.RouteBag();
bag.addRoute( "users/[id]" );
assert( "local alias of a variables array is created", bag.constrainLast( "id", "[a-zA-Z]+" ), 1 );
assert( "mutation through the alias reached the shared array", bag.getRoutes()[ 1 ].constraints.id, "[a-zA-Z]+" );
bag.addRoute( "posts/[slug]" );
assert( "second alias call", bag.constrainLast( "slug", "[0-9]+" ), 2 );
assert( "second mutation visible", bag.getRoutes()[ 2 ].constraints.slug, "[0-9]+" );
assert( "var alias of the shared array reads its length", bag.shareThenShadow(), 2 );
suiteEnd();
</cfscript>
