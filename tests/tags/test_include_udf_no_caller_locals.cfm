<cfscript>
suiteBegin("Function-scoped include: a bare UDF call does not see the caller's locals");

// A template included from inside a method runs in that method's `local`
// scope. A UDF called from that template resolves local -> arguments -> the
// component's variables scope -- it never sees the method's `var`s or the
// include's own `var`s. Unscoped writes inside the include (`args = ...`, the
// for-in loop variable) go to the component's variables scope and so ARE
// visible. Probed on Lucee 7.1 with exactly this fixture:
//   viewpath:unseen|viewlocal:unseen|loopvar:SEEN=1|controller:SEEN=CTRL|argsvar:SEEN
// RustCFML used to carry every local of the include frame into the callee
// (viewpath, viewlocal AND the compiler's loop temporaries) -- a divergence
// that also cost ~7k frames per Preside admin render an inherited-key set.
r = new IncludeUdfRenderer();
out = trim( r.renderView() );
assert( "callee does not see the method's var",   listGetAt(out, 1, "|"), "viewpath:unseen" );
assert( "callee does not see the include's var",  listGetAt(out, 2, "|"), "viewlocal:unseen" );
assert( "unscoped loop var went to variables",    listGetAt(out, 3, "|"), "loopvar:SEEN=1" );
assert( "component variables reachable",           listGetAt(out, 4, "|"), "controller:SEEN=CTRL" );
assert( "unscoped include write went to variables", listGetAt(out, 5, "|"), "argsvar:SEEN" );

suiteEnd();
</cfscript>
