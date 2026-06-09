<cfscript>
suiteBegin("Core: single-part string interpolation preserves the value type");

// ============================================================
// Background
// ============================================================
// On Lucee 5/6/7, Adobe ColdFusion, and BoxLang, a quoted string whose
// ENTIRE content is a single interpolation token -- "#expr#" -- is a
// special case: it does NOT stringify the value, it yields the VALUE
// ITSELF with its type preserved. So `a = "#someStruct#"` leaves `a`
// a struct, `a = "#someArray#"` leaves `a` an array, etc.
//
// As soon as the literal has ANY other part (a leading/trailing
// character, or a second token) it degrades to ordinary string
// concatenation/stringification on every engine -- that is the CONTROL
// below and must pass on BOTH RustCFML and Lucee.
//
// RustCFML 0.92.0 stringifies even the single-token form, so
// isStruct(a) / isArray(a) are false where Lucee reports true. This
// suite documents that gap; the struct and array assertions are the
// discriminators, the controls guard the test wiring.
// ============================================================

// ---- single-token interpolation preserves a STRUCT ----
srcStruct = {x: 1, y: 2};
oneStruct = "#srcStruct#";
assertTrue("single-token '##struct##' stays a struct", isStruct(oneStruct));

// ---- single-token interpolation preserves an ARRAY ----
srcArray = [10, 20, 30];
oneArray = "#srcArray#";
assertTrue("single-token '##array##' stays an array", isArray(oneArray));

// ---- single-token interpolation preserves a NUMBER as numeric ----
srcNum = 42;
oneNum = "#srcNum#";
assertTrue("single-token '##number##' stays numeric", isNumeric(oneNum));

// ============================================================
// CONTROL -- multi-part interpolation stringifies/concatenates on BOTH
// engines. These must pass everywhere; they guard the test wiring.
// ============================================================
twoTokens = "#1##2#";                 // adjacent tokens, no separator
assert("two adjacent tokens concatenate to a string", twoTokens, "12");

dashed = "#1#-#2#";                    // tokens with a literal separator
assert("tokens with a separator concatenate to a string", dashed, "1-2");

leading = "x#srcNum#";                 // leading literal char forces a string
assertTrue("a leading literal char yields a plain string", isSimpleValue(leading));
assert("...and stringifies the value", leading, "x42");

suiteEnd();
</cfscript>
