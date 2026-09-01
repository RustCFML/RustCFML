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

// --- control flow out of a streaming loop (GH #367) ---
// The loop now pumps a VFS line cursor rather than iterating a materialised
// array, and its two exits leave the operand stack in different states: the
// natural end has the EOF sentinel still on it and `break` does not. Getting
// that wrong corrupts the stack for everything AFTER the loop rather than
// failing inside it, so assert both exits and then keep using the frame.
breakLines = [];
loop file=loopFilePath item="bLine" {
    arrayAppend(breakLines, bLine);
    if (arrayLen(breakLines) == 2) { break; }
}
assert("break leaves the loop early", arrayToList(breakLines, "|"), "alpha|beta");

continueLines = [];
loop file=loopFilePath item="cLine" {
    if (len(cLine) == 0) { continue; }
    arrayAppend(continueLines, cLine);
}
assert("continue skips an iteration", arrayToList(continueLines, "|"), "alpha|beta|gamma");

// A plain expression after both loops: evaluates to garbage, not 3, if either
// exit path left the stack unbalanced.
assert("stack is balanced after break and continue", 1 + 2, 3);

// --- nesting: two live cursors at once, inner re-opened per outer line ---
nested = [];
loop file=loopFilePath item="outerLine" {
    loop file=loopFilePath item="innerLine" {
        if (len(outerLine) && len(innerLine)) { arrayAppend(nested, outerLine & ":" & innerLine); }
    }
}
assert("nested file loops each iterate fully", arrayLen(nested), 9);
assert("nested file loops interleave correctly", nested[1] & "/" & nested[9], "alpha:alpha/gamma:gamma");

// --- edge shapes ---
emptyPath = getTempDirectory() & "rcf_loop_file_empty_" & createUUID() & ".txt";
fileWrite(emptyPath, "");
emptyCount = 0;
loop file=emptyPath item="eLine" { emptyCount++; }
assert("an empty file yields no lines", emptyCount, 0);
fileDelete(emptyPath);

// No trailing newline, and a CRLF terminator: both must behave as str::lines
// does -- terminator stripped, no phantom final empty line.
crlfPath = getTempDirectory() & "rcf_loop_file_crlf_" & createUUID() & ".txt";
fileWrite(crlfPath, "one" & chr(13) & chr(10) & "two");
crlfLines = [];
loop file=crlfPath item="rLine" { arrayAppend(crlfLines, rLine); }
assert("CRLF terminator is stripped and no phantom trailing line", arrayToList(crlfLines, "|"), "one|two");
fileDelete(crlfPath);

// A file with more lines than a loop body typically sees, iterated twice, to
// catch a cursor that is not reset or not released between loops.
manyPath = getTempDirectory() & "rcf_loop_file_many_" & createUUID() & ".txt";
manyBuf = [];
for (mi = 1; mi <= 500; mi++) { arrayAppend(manyBuf, "line-" & mi); }
fileWrite(manyPath, arrayToList(manyBuf, chr(10)));
manyFirst = 0;
loop file=manyPath item="mLine" { manyFirst++; }
manySecond = 0;
loop file=manyPath item="mLine2" { manySecond++; }
assert("a multi-line file iterates fully", manyFirst, 500);
assert("re-opening the same file iterates fully again", manySecond, 500);
fileDelete(manyPath);

// --- loop-variable spellings agree with the array form (GH #367) ---
// The streaming loop assigns the binding through the ordinary assignment path
// rather than the array for-in's own store logic, so assert the two agree on
// the spellings where they could diverge: a member path, and a scoped name.
ctx = {};
loop file=loopFilePath item="ctx.item" { }
arrCtx = {};
loop array=scriptItem item="arrCtx.item" { }
assert("member-path binding matches the array form", ctx.item, arrCtx.item);
assert("member-path binding holds the last line", ctx.item, "gamma");

// A `return` out of the loop body: the cursor holds a file descriptor, so the
// close has to happen on this exit too. Called enough times that a leaked
// descriptor per call would exhaust the process limit rather than pass quietly.
function firstLineOf( required string f ) {
    loop file=arguments.f item="fLine" { return fLine; }
    return "";
}
returnOk = 0;
for (ri = 1; ri <= 2000; ri++) { if (firstLineOf(loopFilePath) == "alpha") { returnOk++; } }
assert("return out of a file loop closes the cursor (2000x)", returnOk, 2000);

// Same for an exception leaving the body.
function throwOutOf( required string f ) {
    try { loop file=arguments.f item="tLine" { throw(type="custom", message="stop"); } }
    catch (any e) { return "caught"; }
    return "no";
}
throwOk = 0;
for (ti = 1; ti <= 2000; ti++) { if (throwOutOf(loopFilePath) == "caught") { throwOk++; } }
assert("an exception out of a file loop closes the cursor (2000x)", throwOk, 2000);

// A missing file must throw, not iterate zero times and look like an empty file.
assertThrows("a missing file throws rather than silently iterating nothing", function() {
    loop file=getTempDirectory() & "rcf_loop_file_absent_" & createUUID() & ".txt" item="xLine" {
        writeOutput(xLine);
    }
});

fileDelete(loopFilePath);
suiteEnd();
</cfscript>
