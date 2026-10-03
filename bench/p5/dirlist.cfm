<cfscript>
dir = "/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website/preside/system/assets";
function t(label, fn) { var t0 = getTickCount("nano"); var r = ""; for (var i=1;i<=5;i++) { r = fn(); } writeOutput(label & ": " & numberFormat((getTickCount("nano")-t0)/1000/5,"0") & " us, rows=" & (isQuery(r) ? r.recordCount : arrayLen(r)) & chr(10)); }
t("query recurse *.js sort lastmod", function(){ return DirectoryList(dir, true, "query", "*.js", "DateLastModified"); });
t("query recurse * sort lastmod",    function(){ return DirectoryList(dir, true, "query", "*", "DateLastModified"); });
t("query recurse * nosort",          function(){ return DirectoryList(dir, true, "query", "*"); });
t("path recurse * nosort",           function(){ return DirectoryList(dir, true, "path", "*"); });
</cfscript>
