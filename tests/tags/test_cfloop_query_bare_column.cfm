<cfscript>
suiteBegin("Tags: cfloop query bare column references");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    Inside <cfloop query="q">, a BARE column reference -- `grp` in an
    expression, or ##grp## interpolated in the body -- must resolve to the
    CURRENT ROW's column value: Lucee puts the query at the head of the scope
    cascade for the loop body, exactly as it does for <cfoutput query> (which
    RustCFML already supports -- known-issues §13: bare column refs "resolved
    by merging each row into the variables scope"). Verified on Lucee 7: the
    bare and scoped forms agree.

    RustCFML resolves bare columns in <cfoutput query> but NOT in
    <cfloop query>: the bare read throws "Variable 'grp' is undefined" while
    the scoped q.grp spelling works (control below). Runtime-level gap (a
    catchable throw, no parse error), so it is pinned inline under cftry and
    registration is runner-safe.

    Reduced from the titan (Moopa) codebase port: legacy report templates
    iterate line items with <cfloop query> bodies written with bare column
    names throughout.
    ============================================================
--->

<cfset q = queryNew("grp,val", "varchar,integer",
    [["A",1],["A",2],["B",3],["B",4],["B",5],["C",6]])>

<!--- Control: the scoped q.grp spelling resolves per row today. --->
<cfset scopedSeq = "">
<cfloop query="q"><cfset scopedSeq = scopedSeq & q.grp></cfloop>
<cfscript>
assert("control: scoped q.grp resolves per row", scopedSeq, "AABBBC");
</cfscript>

<!--- Gap 1: a bare column read in an expression inside the loop body. --->
<cfset bareSeq = "">
<cfset bareErr = "">
<cftry>
    <cfloop query="q"><cfset bareSeq = bareSeq & grp></cfloop>
    <cfcatch type="any"><cfset bareErr = cfcatch.message></cfcatch>
</cftry>
<cfscript>
assert("bare column read in a loop-body expression does not throw", bareErr, "");
assert("bare column read agrees with the scoped form", bareSeq, "AABBBC");
</cfscript>

<!--- Gap 2: the same bare column interpolated in output inside the loop. --->
<cfset bareOut = "">
<cfset bareOutErr = "">
<cftry>
    <cfsavecontent variable="bareOut"><cfloop query="q"><cfoutput>#grp#-#val# </cfoutput></cfloop></cfsavecontent>
    <cfcatch type="any"><cfset bareOutErr = cfcatch.message></cfcatch>
</cftry>
<cfscript>
assert("bare column interpolation in the loop body does not throw", bareOutErr, "");
assert("bare column interpolation renders per row", trim(bareOut), "A-1 A-2 B-3 B-4 B-5 C-6");

suiteEnd();
</cfscript>
