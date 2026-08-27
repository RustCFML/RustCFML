<cfscript>
suiteBegin("Loop: file= iterates a file's lines in the script form as well as the tag form");

// <cfloop file="..." index="..."> has iterated a file line by line since
// GH #158, but the SCRIPT spelling -- `loop file=p item="line" { ... }`,
// the second of the two forms in Lucee's "loop through files" recipe --
// had no `file` branch at all. It fell through to the infinite-loop
// fallback, so the body ran with the binding never created and threw
// "Variable 'line' is undefined" (GH #367).
//
// Both spellings lower to the same iteration, so the legs below assert the
// two produce identical results rather than just "the script form runs".

loopFilePath = getTempDirectory() & "rcf_loop_file_" & createUUID() & ".txt";
// A blank line in the middle: line-based iteration must preserve it, not
// collapse it the way a listToArray() over chr(10) would.
fileWrite(loopFilePath, "alpha" & chr(10) & "beta" & chr(10) & chr(10) & "gamma");

// --- script form, item= (the spelling in the report) ---
scriptItem = [];
loop file=loopFilePath item="loopLine" {
    arrayAppend(scriptItem, loopLine);
}
assert("script form, item=: every line including the blank one", arrayToList(scriptItem, "|"), "alpha|beta||gamma");
assert("script form, item=: line count", arrayLen(scriptItem), 4);

// --- script form, index= (the legacy spelling of the same binding) ---
scriptIndex = [];
loop file=loopFilePath index="loopLine2" {
    arrayAppend(scriptIndex, loopLine2);
}
assert("script form, index= binds the line too", arrayToList(scriptIndex, "|"), "alpha|beta||gamma");
</cfscript>

<!--- --- tag form: must agree with the script form exactly --- --->
<cfset tagLines = []>
<cfloop file="#loopFilePath#" index="tagLine"><cfset arrayAppend(tagLines, tagLine)></cfloop>

<cfscript>
assert("tag form agrees with the script form", arrayToList(tagLines, "|"), arrayToList(scriptItem, "|"));

// The binding survives the loop with the last line's value (both engines).
assert("binding holds the final line after the loop", loopLine, "gamma");

fileDelete(loopFilePath);
suiteEnd();
</cfscript>
