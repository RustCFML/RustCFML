<cfscript>
suiteBegin("A method-include mixin never shadows an existing method (Preside presideProxies shape)");

// The exact shape that sent Preside admin login into "infinite recursion
// detected: renderView (depth 256)": ColdBox mixes presideProxies.cfm into the
// Renderer; the helper file declares renderView, which proxies BACK to the
// renderer, whose class also declares renderView. The class method must keep
// winning member dispatch; the mixin stays reachable through `variables`.
// (An isolated Lucee probe lets the mixin shadow the method and recurses on
// this very shape, so this file is RustCFML-only — our rule is the one that
// keeps Preside alive.)
r = new oop.fixtures.gh481.MixinRenderer();
request.gh481Renderer = r;
r.init();
assert("the class method wins member dispatch", r.gh481RenderView( "home" ), "REAL[home]");
structDelete( request, "gh481Renderer" );

suiteEnd();
</cfscript>
