<cfscript>
function includeUdfHelper() {
    var r = [];
    r.append("viewpath:" & (isDefined("viewpath") ? "SEEN" : "unseen"));
    r.append("viewlocal:" & (isDefined("viewlocal") ? "SEEN" : "unseen"));
    r.append("loopvar:" & (isDefined("i") ? "SEEN=" & i : "unseen"));
    r.append("controller:" & (isDefined("controller") ? "SEEN=" & controller : "unseen"));
    r.append("argsvar:" & (isDefined("args") ? "SEEN" : "unseen"));
    return r.toList("|");
}
</cfscript>
