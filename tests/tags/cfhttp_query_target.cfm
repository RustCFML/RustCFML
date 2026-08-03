<cfscript>
// Target page for the <cfhttp name=/file=> probes and tests. Emits delimited
// text bodies (for name= query parsing) plus redirect/404 modes.
param name="url.mode" default="csv";
lf = chr(10);
switch (url.mode) {
    case "csv":
        content type="text/plain";
        writeOutput("name,age" & lf & "alice,30" & lf & "bob,25");
        break;
    case "pipe":
        content type="text/plain";
        writeOutput("name|age" & lf & "alice|30" & lf & "bob|25");
        break;
    case "quoted":
        content type="text/plain";
        writeOutput("name,note" & lf & '"alice","likes, commas"' & lf & '"bob","plain"');
        break;
    case "noheader":
        content type="text/plain";
        writeOutput("alice,30" & lf & "bob,25");
        break;
    case "zeros":
        content type="text/plain";
        writeOutput("code,qty" & lf & "007,1" & lf & "008,2");
        break;
    case "ragged":
        content type="text/plain";
        writeOutput("a,b,c" & lf & "1,2" & lf & "4,5,6,7");
        break;
    case "escaped":
        content type="text/plain";
        writeOutput("a,b" & lf & '"say ""hi""",2');
        break;
    case "blanks":
        content type="text/plain";
        writeOutput("a,b" & lf & lf & "1,2" & lf);
        break;
    case "redirect":
        location url="/tests/tags/cfhttp_query_target.cfm?mode=csv" statuscode="302" addtoken="false";
        break;
    case "notfound":
        header statuscode="404" statustext="Not Found";
        content type="text/plain";
        writeOutput("nope");
        break;
    default:
        writeOutput("unknown-mode");
}
</cfscript>