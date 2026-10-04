<cfscript>
o = new VX(); N = 2000000;
function t(label, m) { o[m](1000); var t0 = getTickCount("nano"); invoke(o, m, {n=N}); writeOutput(label & ": " & numberFormat((getTickCount("nano")-t0)/N, "0") & " ns" & chr(10)); }
t("component variables.x = i      ", "setX"); t("component variables.cache.k = i", "setCacheK"); t("component empty loop          ", "loopOnly");
</cfscript>
