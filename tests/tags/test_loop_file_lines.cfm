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

<cfscript>
suiteBegin("Loop: file= honours a line window (startLine/endLine, a.k.a. from/to)");

// Reading only a file's header to validate it is the reason `loop file=`
// takes a line window: without one the only way to stop was `break`, and
// the whole point of the streaming loop is not to touch what you did not
// ask for (GH #367 follow-up).
//
// Lucee accepts BOTH spellings -- `startLine`/`endLine` are the documented
// attribute names, `from`/`to` are aliases -- and everything asserted below
// was verified against Lucee 7.1 before it was implemented here.

winPath = getTempDirectory() & "rcf_loop_window_" & createUUID() & ".txt";
winBuf = [];
for (wi = 1; wi <= 10; wi++) { arrayAppend(winBuf, "line" & wi); }
fileWrite(winPath, arrayToList(winBuf, chr(10)));

// Collect the lines a window yields, as a pipe-delimited string.
function winLines( required string path, struct attrs = {} ) {
    var out = [];
    var a = arguments.attrs;
    if ( structKeyExists(a, "startline") && structKeyExists(a, "endline") ) {
        loop file=arguments.path item="wLine" startline=a.startline endline=a.endline { arrayAppend(out, wLine); }
    } else if ( structKeyExists(a, "from") && structKeyExists(a, "to") ) {
        loop file=arguments.path item="wLine" from=a.from to=a.to { arrayAppend(out, wLine); }
    } else if ( structKeyExists(a, "from") ) {
        loop file=arguments.path item="wLine" from=a.from { arrayAppend(out, wLine); }
    } else if ( structKeyExists(a, "to") ) {
        loop file=arguments.path item="wLine" to=a.to { arrayAppend(out, wLine); }
    } else {
        loop file=arguments.path item="wLine" { arrayAppend(out, wLine); }
    }
    return arrayToList(out, "|");
}

// --- the reported case: read just the header ---
assert("from=1 to=1 yields only the first line", winLines(winPath, {from: 1, to: 1}), "line1");

// --- both spellings, and each bound on its own ---
assert("from/to select an interior window", winLines(winPath, {from: 2, to: 4}), "line2|line3|line4");
assert("startLine/endLine select the same window", winLines(winPath, {startline: 2, endline: 4}), "line2|line3|line4");
assert("from alone runs to EOF", winLines(winPath, {from: 8}), "line8|line9|line10");
assert("to alone starts at line 1", winLines(winPath, {to: 2}), "line1|line2");
assert("no window still yields every line", listLen(winLines(winPath), "|"), 10);

// --- out-of-range and inverted bounds: no iterations, never an error ---
// (Lucee clamps a start below 1 to the first line and runs the body no times
// when the window is empty; a start past EOF is not an error either.)
assert("from below 1 clamps to the first line", winLines(winPath, {from: -3, to: 2}), "line1|line2");
assert("from=0 clamps to the first line", winLines(winPath, {startline: 0, endline: 2}), "line1|line2");
assert("to=0 yields nothing", winLines(winPath, {from: 1, to: 0}), "");
assert("to below from yields nothing", winLines(winPath, {from: 5, to: 2}), "");
assert("from past EOF yields nothing", winLines(winPath, {from: 50, to: 60}), "");
assert("to past EOF stops at EOF", winLines(winPath, {from: 9, to: 50}), "line9|line10");

// --- coercion: numeric strings work, fractions truncate, junk throws ---
assert("numeric strings are accepted", winLines(winPath, {from: "2", to: "3"}), "line2|line3");
assert("a fractional to truncates", winLines(winPath, {from: 1, to: 3.7}), "line1|line2|line3");
assert("a fractional from truncates", winLines(winPath, {from: 2.7, to: 4}), "line2|line3|line4");
assertThrows("a non-numeric bound throws rather than reading nothing", function() {
    loop file=winPath item="badLine" from="abc" to=2 { writeOutput(badLine); }
});

// --- startLine/endLine win over the from/to aliases when both are given ---
bothWays = [];
loop file=winPath item="bwLine" from=2 to=3 startline=6 endline=8 { arrayAppend(bothWays, bwLine); }
assert("startLine/endLine take precedence over from/to", arrayToList(bothWays, "|"), "line6|line7|line8");

// --- index= is the same binding as item= here too ---
idxWin = [];
loop file=winPath index="iwLine" from=3 to=4 { arrayAppend(idxWin, iwLine); }
assert("index= binds a windowed line as well", arrayToList(idxWin, "|"), "line3|line4");

// A file loop with from/to and `index` must NOT be read as the counted
// `from`/`to`/`index` loop -- that lowering ignores `file` entirely and binds
// the COUNTER, so the body saw 3 and 4 instead of the file's lines.
assertFalse("a windowed file loop binds lines, not the counter", isNumeric(idxWin[1]));
</cfscript>

<!--- --- tag form: must agree with the script form on both spellings --- --->
<cfset tagWinFromTo = []>
<cfloop file="#winPath#" index="twLine" from="3" to="5"><cfset arrayAppend(tagWinFromTo, twLine)></cfloop>
<cfset tagWinStartEnd = []>
<cfloop file="#winPath#" index="tsLine" startline="3" endline="5"><cfset arrayAppend(tagWinStartEnd, tsLine)></cfloop>
<cfset tagWinOpenEnd = []>
<cfloop file="#winPath#" index="toLine" startline="9"><cfset arrayAppend(tagWinOpenEnd, toLine)></cfloop>

<cfscript>
assert("tag form from/to windows the file", arrayToList(tagWinFromTo, "|"), "line3|line4|line5");
assert("tag form startLine/endLine agrees", arrayToList(tagWinStartEnd, "|"), arrayToList(tagWinFromTo, "|"));
assert("tag form startLine alone runs to EOF", arrayToList(tagWinOpenEnd, "|"), "line9|line10");

// --- break and continue still work inside a window, and the window still caps ---
winBreak = [];
loop file=winPath item="wbLine" from=2 to=8 {
    arrayAppend(winBreak, wbLine);
    if (arrayLen(winBreak) == 2) { break; }
}
assert("break inside a window leaves early", arrayToList(winBreak, "|"), "line2|line3");

winCont = [];
loop file=winPath item="wcLine" from=2 to=5 {
    if (wcLine == "line3") { continue; }
    arrayAppend(winCont, wcLine);
}
assert("continue inside a window skips one line", arrayToList(winCont, "|"), "line2|line4|line5");
assert("stack is balanced after a windowed break and continue", 1 + 2, 3);

// --- a window into a larger file, twice, so a cursor that is not released
//     between loops shows up ---
bigPath = getTempDirectory() & "rcf_loop_window_big_" & createUUID() & ".txt";
bigBuf = [];
for (bi = 1; bi <= 500; bi++) { arrayAppend(bigBuf, "row-" & bi); }
fileWrite(bigPath, arrayToList(bigBuf, chr(10)));
bigWin = [];
loop file=bigPath item="bgLine" from=200 to=210 { arrayAppend(bigWin, bgLine); }
assert("a window into a 500-line file yields exactly its lines", arrayLen(bigWin), 11);
assert("the window starts and ends where asked", bigWin[1] & "/" & bigWin[11], "row-200/row-210");
bigAgain = [];
loop file=bigPath item="bgLine2" from=200 to=210 { arrayAppend(bigAgain, bgLine2); }
assert("re-running the same window gives the same lines", arrayToList(bigAgain, "|"), arrayToList(bigWin, "|"));
fileDelete(bigPath);

// A `return` out of a windowed loop closes the cursor like the unwindowed one
// does -- the window changes when the loop ends, not who owns the descriptor.
function headerOf( required string f ) {
    loop file=arguments.f item="hLine" from=1 to=1 { return hLine; }
    return "";
}
headerOk = 0;
for (hi = 1; hi <= 2000; hi++) { if (headerOf(winPath) == "line1") { headerOk++; } }
assert("return out of a windowed loop closes the cursor (2000x)", headerOk, 2000);

fileDelete(winPath);
suiteEnd();
</cfscript>
