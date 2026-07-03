<cfscript>
// Report, in order:
//  1. PC read-back value          -> "written-in-pc"
//  2. PC structKeyExists result   -> "SKE-TRUE"
//  3. PC Preside guard-set-return -> "built"
//  4. persistence into page body  -> "NOT-PERSISTED" (Lucee parity: PC writes dropped)
writeOutput( request.pc.readback );
writeOutput( "|" & ( request.pc.ske ? "SKE-TRUE" : "SKE-FALSE" ) );
writeOutput( "|" & request.pc.preside );
writeOutput( "|" & ( structKeyExists( application, "pcVal" ) ? application.pcVal : "NOT-PERSISTED" ) );
</cfscript>
