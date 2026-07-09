<cfscript>
suiteBegin( "Scope mutator via a scope-held receiver does not resurrect it" );

// Receiver lives IN the application scope and is invoked through an
// application-rooted path — the exact pattern that triggered the resurrection.
application.__resurrectProbe    = new oop.ScopeSelfRemover();
application.__resurrectSibling  = "present";

application.__resurrectProbe.removeMeFromApplication( "__resurrectProbe" );

// Pre-fix: the this-writeback did deep_set(application, ["__resurrectProbe"], ...)
// AFTER the method deleted it, resurrecting the receiver.
assertFalse( "receiver not resurrected after self-removal", structKeyExists( application, "__resurrectProbe" ) );

// A sibling key the method never touched is unaffected.
assertTrue( "sibling key untouched", structKeyExists( application, "__resurrectSibling" ) );

// housekeeping so the shared runner's application scope is left clean
structDelete( application, "__resurrectSibling" );

suiteEnd();
</cfscript>
