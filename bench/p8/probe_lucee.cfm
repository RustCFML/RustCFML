<cfscript>
lib = "/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website/preside/system/services/security/antisamylib/";
try { x = createObject( "java", "org.owasp.validator.html.AntiSamy", [ lib & "antisamy-1.5.3.jar" ] ); writeOutput("created: " & isNull(x) & " / " & (isNull(x) ? "" : x.getClass().getName()) & chr(10)); } catch (any e) { writeOutput("ERR " & e.message & chr(10)); }
</cfscript>
