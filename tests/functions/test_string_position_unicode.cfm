<!---
  GitHub #248: positional string BIFs (Find/FindNoCase/REFind/REFindNoCase,
  and Insert) returned/consumed UTF-8 BYTE offsets while Mid/Left/Right/Len are
  CHARACTER-based. On any string with a non-ASCII (multibyte) char the two units
  disagreed, so the ubiquitous mid(s, find(needle, s), n) idiom sliced the wrong
  substring (and find() could return a position > len(s)).

  Fix: Find/FindNoCase/REFind/REFindNoCase now return 1-based CHARACTER
  positions and interpret the start-offset argument as a character offset;
  REFind's returnsubexpressions pos/len arrays are character-based too; Insert's
  position is a character offset. This makes positions produced by one BIF valid
  inputs to every other, matching Lucee 7 for the BMP.

  U+2014 EM DASH (chr(8212)) is 1 character but 3 bytes in UTF-8.
--->
<cfscript>
suiteBegin("String position BIFs are character-based (GitHub 248)");

s = "ab" & chr(8212) & "cdXef";
// chars:  a(1) b(2) —(3) c(4) d(5) X(6) e(7) f(8)

assert("len counts characters", len(s), 8);
assert("find returns character position", find("X", s), 6);
assert("find position feeds mid correctly", mid(s, find("X", s), 1), "X");
assert("findNoCase character position", findNoCase("x", s), 6);
assert("reFind character position", reFind("X", s), 6);
assert("reFindNoCase character position", reFindNoCase("x", s), 6);

// 3-arg find with a start offset AFTER a multibyte char
s2 = "X" & chr(8212) & "X";
// chars: X(1) —(2) X(3)
assert("find 3-arg start offset is character-based", find("X", s2, 2), 3);
assert("reFind 3-arg start offset is character-based", reFind("X", s2, 2), 3);

// reFind returnsubexpressions position & length are character-based
st = reFind("X", s, 1, true);
assert("reFind returnsubexpressions pos is char-based", st.pos[1], 6);
assert("reFind returnsubexpressions len is char-based", st.len[1], 1);

// A multi-char match's length is counted in characters, not bytes
stm = reFind(chr(8212) & "cd", s, 1, true);
assert("reFind multi-char match len is char count", stm.len[1], 3);
assert("reFind multi-char match pos is char-based", stm.pos[1], 3);

// The killer idiom end-to-end
assert("mid(s, find, 3) extracts intended slice", mid(s, find("X", s), 3), "Xef");

// find never exceeds len(s)
assert("find result never exceeds len", find("f", s) <= len(s), true);
assert("find last char position", find("f", s), 8);

// Insert uses a character offset (and must not corrupt multibyte input)
assert("insert after 3 chars past a multibyte char",
       insert("!", "ab" & chr(8212) & "cd", 3), "ab" & chr(8212) & "!cd");
assert("insert at start", insert("!", "a" & chr(8212) & "b", 0), "!a" & chr(8212) & "b");
assert("insert at end", insert("!", "a" & chr(8212) & "b", 3), "a" & chr(8212) & "b!");

// Not-found still returns 0
assert("find not found returns 0", find("Z", s), 0);
assert("reFind not found returns 0", reFind("Z", s), 0);

// Pure-ASCII behavior is unchanged (regression guard)
assert("ascii find", find("cd", "abcdef"), 3);
assert("ascii findNoCase", findNoCase("CD", "abcdef"), 3);
assert("ascii reFind", reFind("cd", "abcdef"), 3);
assert("ascii mid via find", mid("abcdef", find("cd", "abcdef"), 2), "cd");

suiteEnd();
</cfscript>
