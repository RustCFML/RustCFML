<cfscript>
suiteBegin("component name with a doubled dot resolves to ONE canonical file (GH profile: system//handlers)");

// ColdBox builds `HandlersInvocationPath` as appMappingAsDots & ".handlers"; when the
// app mapping ends in a slash that is `preside.system..handlers.X`. Lucee collapses it.
// RustCFML resolved the file fine but kept `system//handlers/X.cfc` as the class's
// identity, so every handler was compiled, class-built and profiled TWICE.
clean  = getComponentMetaData( "oop.GcmCacheL1" ).path;
dotted = getComponentMetaData( "oop..GcmCacheL1" ).path;
assertFalse( "doubled-dot name yields no doubled slash in path", find( "//", dotted ) > 0 );
assert( "doubled-dot name resolves to the same file as the clean name", dotted, clean );
inst = getMetaData( createObject( "component", "oop..GcmCacheL1" ) ).path;
assert( "instance created via the doubled-dot name reports the clean path", inst, clean );
assert( "the doubled-dot instance still inherits", getMetaData( createObject( "component", "oop..GcmCacheL1" ) ).extends.name, "oop.GcmCacheL2" );

suiteEnd();
</cfscript>
