<cfscript>
// Report, in order:
//  1. PC read-back value          -> "written-in-pc"
//  2. PC structKeyExists result   -> "SKE-TRUE"
//  3. PC Preside guard-set-return -> "built"
//  4. persistence into page body  -> "NOT-PERSISTED" (Lucee parity: page sees the
//                                    named scope, never the PC's)
//  5. did the PC see the PREVIOUS request's PC write -> "SAW-PREV" / "NO-PREV"
//  6. did the guard-once branch execute this request -> "GUARD-RAN" / "GUARD-SKIPPED"
writeOutput( request.pc.readback );
writeOutput( "|" & ( request.pc.ske ? "SKE-TRUE" : "SKE-FALSE" ) );
writeOutput( "|" & request.pc.preside );
writeOutput( "|" & ( structKeyExists( application, "pcVal" ) ? application.pcVal : "NOT-PERSISTED" ) );
writeOutput( "|" & ( request.pc.sawPrev ? "SAW-PREV" : "NO-PREV" ) );
writeOutput( "|" & ( request.pc.guardRan ? "GUARD-RAN" : "GUARD-SKIPPED" ) );
</cfscript>
