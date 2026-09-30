<cfscript>
if (thisTag.executionMode == "start") {
    caller[attributes.outVar] &= "[start]";
    exit;
    caller[attributes.outVar] &= "[after-start]";
} else {
    caller[attributes.outVar] &= "[end]";
}
</cfscript>
