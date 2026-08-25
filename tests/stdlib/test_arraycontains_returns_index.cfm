<!--- GH #358 — arrayContains / arrayContainsNoCase return the 1-based index of
      the first match and 0 when absent, exactly like arrayFind. They used to
      return a boolean, which reads identically inside an `if` and diverges the
      moment the result is USED: `list[ arrayContains( list, x ) ]` yields the
      element on Lucee and threw here.

      Measured on Lucee 7.1.0.204. Lucee's ArrayContains.call is literally
      `return ArrayFind.call( pc, array, value )`, which is also why the
      closure-predicate and substringMatch forms below behave as they do —
      including the asymmetry that arrayContainsNoCase does NOT take a closure
      (it routes to ArrayFindNoCase, which has no UDF branch). --->
<cfscript>
suiteBegin("stdlib: arrayContains returns an index (GH ##358)");

_nums = [ 65, 66, 67 ];
_strs = [ "a", "B", "c" ];

assert("hit returns the 1-based index", arrayContains(_nums, 66), 2);
assert("miss returns 0", arrayContains(_nums, 99), 0);
assert("the index is usable as a subscript", _nums[ arrayContains(_nums, 67) ], 67);
assert("first of several duplicates", arrayContains([9,9,9], 9), 1);
assert("empty array", arrayContains([], 1), 0);

// It agrees with its sibling arrayFind, which always returned the index.
assert("arrayContains agrees with arrayFind", arrayContains(_nums, 66), arrayFind(_nums, 66));

// Case handling is unchanged — only the return SHAPE moved.
assert("case-sensitive form does not match", arrayContains(_strs, "b"), 0);
assert("NoCase form matches", arrayContainsNoCase(_strs, "b"), 2);
assert("NoCase miss returns 0", arrayContainsNoCase(_strs, "z"), 0);
assert("arrayContainsNoCase agrees with arrayFindNoCase",
	arrayContainsNoCase(_strs, "b"), arrayFindNoCase(_strs, "b"));

// The member forms alias the BIFs, so they moved too.
assert("member .contains()", _nums.contains(66), 2);
assert("member .containsNoCase()", _strs.containsNoCase("b"), 2);

// 0/index still reads correctly as a boolean, which is why this went unnoticed.
assertTrue("a hit is still truthy", arrayContains(_nums, 66) ? true : false);
assertFalse("a miss is still falsy", arrayContains(_nums, 99) ? true : false);

// Complex needles match by identity/deep-equality, and report their index.
_s1 = { x = 1 };
_s2 = { x = 2 };
assert("struct needle returns its index", arrayContains([_s1, _s2], _s2), 2);

// substringMatch (3rd argument) — match when the needle appears INSIDE the
// element's string form. Lucee's plain form is case-SENSITIVE here even though
// its equality scan is not.
_words = [ "alpha", "BETA", "gamma" ];
assert("substringMatch finds a substring", arrayContains(_words, "amm", true), 3);
assert("substringMatch is case-sensitive", arrayContains(_words, "beta", true), 0);
assert("substringMatch NoCase is not", arrayContainsNoCase(_words, "beta", true), 2);
assert("substringMatch miss", arrayContains(_words, "zzz", true), 0);
assert("substringMatch=false is the equality form", arrayContains(_strs, "B", false), 2);
assertThrows("a complex needle with substringMatch is rejected", function() {
	arrayContains(_nums, [1], true);
});

// Closure-predicate needle: arrayContains inherits ArrayFind's UDF branch.
assert("closure predicate returns the first truthy index",
	arrayContains(_nums, function(v) { return v == 67; }), 3);
assert("closure predicate with no match", arrayContains(_nums, function(v) { return false; }), 0);

// Found in the same pass: arrayFindAll / arrayFindAllNoCase with a VALUE needle
// were dispatched past the builtin entirely and threw "function is not
// defined". Only the closure form ever worked.
assert("arrayFindAll with a value needle", arrayToList(arrayFindAll([1,2,1], 1)), "1,3");
assert("arrayFindAllNoCase with a value needle",
	arrayToList(arrayFindAllNoCase(["a","A","b"], "a")), "1,2");
assert("arrayFindAll with a closure",
	arrayToList(arrayFindAll(_nums, function(v) { return v > 65; })), "2,3");

suiteEnd();
</cfscript>
