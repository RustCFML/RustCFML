<cfscript>
suiteBegin("callStackGet / callStackDump");

// --- callStackGet returns array ---
stack = callStackGet();
assertTrue("callStackGet returns array", isArray(stack));
assertTrue("callStackGet has frames", arrayLen(stack) > 0);

// --- Each frame has correct keys ---
frame = stack[1];
assertTrue("frame has Function key", structKeyExists(frame, "Function"));
assertTrue("frame has Template key", structKeyExists(frame, "Template"));
assertTrue("frame has LineNumber key", structKeyExists(frame, "LineNumber"));

// --- Nested function calls show stack ---
function innerFunc() {
    return callStackGet();
}
function outerFunc() {
    return innerFunc();
}
nestedStack = outerFunc();
assertTrue("nested stack has multiple frames", arrayLen(nestedStack) >= 3);
assert("innermost frame is innerFunc", nestedStack[1].Function, "innerFunc");
assert("next frame is outerFunc", nestedStack[2].Function, "outerFunc");

// --- callStackGet("array") returns array format ---
function getStackAsArray() {
    return callStackGet("array");
}
arrayStack = getStackAsArray();
assertTrue("callStackGet array format", isArray(arrayStack));
assertTrue("callStackGet array has frames", arrayLen(arrayStack) >= 1);

// --- callStackGet("string") returns a STRING (Lucee parity) ---
// Preside's datamanager _objectDataTable.cfm concatenates this into a hash
// seed (`... & CallStackGet("string") & ...`); returning the array 500'd the
// admin listing with "Can't cast Array to String".
function getStackAsString() {
    return callStackGet("string");
}
stringStack = getStackAsString();
assertTrue("callStackGet('string') returns a simple value", isSimpleValue(stringStack));
assertTrue("callStackGet('string') is non-empty", len(stringStack) > 0);
// It must concatenate cleanly with & (the exact failure Preside hit).
assertTrue("callStackGet('string') concatenates with &", len("x" & callStackGet("string") & "y") > 2);
// Frames are separated by "; " and name the enclosing function.
assertTrue("callStackGet('string') names the function", findNoCase("getStackAsString", stringStack) > 0);

// --- callStackGet("html") returns an HTML list ---
htmlStack = callStackGet("html");
assertTrue("callStackGet('html') is a simple value", isSimpleValue(htmlStack));
assertTrue("callStackGet('html') is a <ul> list", findNoCase("<ul", htmlStack) > 0 && findNoCase("<li>", htmlStack) > 0);

// --- callStackGet("json") returns valid JSON that deserializes to an array ---
jsonStack = callStackGet("json");
assertTrue("callStackGet('json') is a simple value", isSimpleValue(jsonStack));
assertTrue("callStackGet('json') is valid JSON", isJSON(jsonStack));
assertTrue("callStackGet('json') deserializes to an array", isArray(deserializeJSON(jsonStack)));

suiteEnd();
</cfscript>
