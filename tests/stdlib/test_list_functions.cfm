<cfscript>
suiteBegin("List Functions");

// --- listLen ---
assert("listLen a,b,c", listLen("a,b,c"), 3);
assert("listLen with pipe delim", listLen("a|b|c", "|"), 3);

// --- listFirst / listLast / listRest ---
assert("listFirst", listFirst("a,b,c"), "a");
assert("listLast", listLast("a,b,c"), "c");
assert("listRest", listRest("a,b,c"), "b,c");

// --- listGetAt ---
assert("listGetAt pos 2", listGetAt("a,b,c", 2), "b");

// --- listSetAt ---
assert("listSetAt pos 2 to X", listSetAt("a,b,c", 2, "X"), "a,X,c");

// --- listDeleteAt ---
assert("listDeleteAt pos 2", listDeleteAt("a,b,c", 2), "a,c");

// --- listAppend ---
assert("listAppend c", listAppend("a,b", "c"), "a,b,c");

// --- listPrepend ---
assert("listPrepend a", listPrepend("b,c", "a"), "a,b,c");

// --- listInsertAt ---
assert("listInsertAt pos 2", listInsertAt("a,c", 2, "b"), "a,b,c");

// --- listFind ---
assert("listFind b found", listFind("a,b,c", "b"), 2);
assert("listFind d not found", listFind("a,b,c", "d"), 0);

// --- listFindNoCase ---
assert("listFindNoCase b in A,B,C", listFindNoCase("A,B,C", "b"), 2);

// --- listContains ---
assertTrue("listContains app in apple,banana", listContains("apple,banana", "app") > 0);

// --- listContainsNoCase ---
assertTrue("listContainsNoCase apple in Apple,Banana", listContainsNoCase("Apple,Banana", "apple") > 0);

// --- listSort ---
assert("listSort text asc", listSort("c,a,b", "text"), "a,b,c");

// --- listRemoveDuplicates ---
deduped = listRemoveDuplicates("a,b,a,c");
assert("listRemoveDuplicates len", listLen(deduped), 3);

// --- listToArray ---
arr = listToArray("a,b,c");
assert("listToArray arrayLen", arrayLen(arr), 3);

// --- listValueCount ---
assert("listValueCount a appears 2x", listValueCount("a,b,a,c", "a"), 2);

// --- listQualify ---
qualified = listQualify("a,b,c", "'");
assert("listQualify wraps in quotes", qualified, "'a','b','c'");

// --- listChangeDelims ---
assert("listChangeDelims to pipe", listChangeDelims("a,b,c", "|"), "a|b|c");

// --- listCompact ---
compacted = listCompact("a,,b,,c");
assert("listCompact removes empties", listLen(compacted), 3);

// --- listItemTrim ---
trimmed = listItemTrim(" a , b , c ");
assert("listItemTrim trims items", listFirst(trimmed), "a");

// --- listToArray with EMPTY delimiter (Lucee splits into characters) ---
// This is specific to listToArray; listLen/listFirst/listGetAt instead treat
// an empty delimiter as "no delimiter" (whole string = one element).
assert("listToArray empty-delim splits chars", serializeJson(listToArray("weak", "")), '["w","e","a","k"]');
assert("listToArray empty-delim single char", arrayLen(listToArray("x", "")), 1);
assert("listToArray empty-delim empty string", arrayLen(listToArray("", "")), 0);
assert("listToArray empty-delim keeps unicode", serializeJson(listToArray("a-b", "")), '["a","-","b"]');
// includeEmptyFields=true brackets the chars with a leading + trailing empty
assert("listToArray empty-delim includeEmpty", serializeJson(listToArray("ab", "", true)), '["","a","b",""]');
assert("listToArray empty-delim includeEmpty empty string", serializeJson(listToArray("", "", true)), '[""]');
// empty delimiter stays a no-op for the other list functions
assert("listLen empty-delim is 1", listLen("weak", ""), 1);
assert("listFirst empty-delim whole string", listFirst("weak", ""), "weak");

suiteEnd();
</cfscript>
