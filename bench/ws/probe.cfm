<cfscript>o = new OutFalse(); savecontent variable="c1" { o.f(); } savecontent variable="c2" { o.g(); } savecontent variable="c3" { o.h(); }
writeOutput("f=[" & trim(reReplace(c1,"\s+"," ","all")) & "] g=[" & trim(reReplace(c2,"\s+"," ","all")) & "] h=[" & trim(reReplace(c3,"\s+"," ","all")) & "]");</cfscript>
