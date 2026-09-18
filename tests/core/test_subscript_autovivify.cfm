<cfscript>
suiteBegin("Core: subscript auto-vivification + verbose operator aliases");

// ------------------------------------------------------------
// Subscript-assigning into a variable that does not yet exist creates it,
// matching Lucee/ACF/BoxLang. A string key vivifies a struct; a numeric
// index vivifies a (1-based, auto-growing) array.
// ------------------------------------------------------------
rcfmlAutoVivStruct["alpha"] = 1;
rcfmlAutoVivStruct["beta"]  = 2;
assertTrue("undefined var subscript-assigned with a string key becomes a struct",
    isStruct(rcfmlAutoVivStruct));
// RustCFML structs are always insertion-ordered (IndexMap); Lucee's plain
// auto-vivified struct is hash-ordered, so key order isn't guaranteed there.
if (isRustCFML()) assert("auto-vivified struct keeps both keys", structKeyList(rcfmlAutoVivStruct), "alpha,beta");

// RESOLVED (GH ##428/##429, was "known divergence item E"): a numeric subscript
// vivifies a STRUCT keyed by the subscript, exactly as Lucee does — not a
// 1-based auto-growing array. Both engines run the asserts below now; the fuller
// treatment, including the array-BIF view of such a struct, is in
// tests/core/test_numeric_subscript_vivifies_struct.cfm.
rcfmlAutoVivArray[3] = "c";
assertTrue("undefined var subscript-assigned with a numeric index becomes a struct",
    isStruct(rcfmlAutoVivArray));
assertFalse("...and is NOT an array", isArray(rcfmlAutoVivArray));
assert("the subscript is the key", rcfmlAutoVivArray[3], "c");
assert("it holds exactly one entry", structCount(rcfmlAutoVivArray), 1);

// ------------------------------------------------------------
// Verbose, multi-word comparison operator aliases (Lucee/ACF/BoxLang).
// ------------------------------------------------------------
assertTrue("IS NOT",                    1 IS NOT 2);
assertTrue("NOT EQUAL",                 1 NOT EQUAL 2);
assertTrue("EQUAL",                     2 EQUAL 2);
assertTrue("GREATER THAN",              5 GREATER THAN 3);
assertTrue("LESS THAN",                 3 LESS THAN 5);
assertTrue("GREATER THAN OR EQUAL TO",  5 GREATER THAN OR EQUAL TO 5);
assertTrue("LESS THAN OR EQUAL TO",     4 LESS THAN OR EQUAL TO 4);
assertTrue("DOES NOT CONTAIN",          "abc" DOES NOT CONTAIN "z");

// The operator words must remain usable as ordinary identifiers.
greater = 7; than = 8; equal = 9; less = 10; does = 11; contain = 12;
assert("operator words still usable as variable names",
    greater & than & equal & less & does & contain, "789101112");

suiteEnd();
</cfscript>
