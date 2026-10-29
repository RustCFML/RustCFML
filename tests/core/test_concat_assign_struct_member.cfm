<cfscript>
suiteBegin("&= on a struct member");

// `s.x &= v` moves the string out of the struct so the append happens in
// place; `variables.s &= "x"` used to copy the whole string on every append.
// These pin that the move is invisible.
function br_appendLoop(n) {
	variables.br_cs = "";
	for (var i = 1; i <= n; i++) { variables.br_cs &= "x"; }
	return len(variables.br_cs);
}
assert("variables.x &= in a loop", br_appendLoop(1000), 1000);

st = { s: "ab" };
st.s &= "cd";
assert("plain struct member", st.s, "abcd");
st.s &= 5;
assert("numeric right operand", st.s, "abcd5");
alias = st;
alias.s &= "!";
assert("an alias sees the append", st.s, "abcd5!");

// A complex right operand makes Concat throw: the member keeps its value.
try { st.s &= [1]; } catch (any e) {}
assert("a failed append leaves the member intact", st.s, "abcd5!");

// A member that holds a number is appended to as a string, as before.
st.n = 12;
st.n &= "3";
assert("number member", st.n, "123");

// The same string held in two structs is not shared after the append.
a = { v: "same" };
b = { v: a.v };
a.v &= "-a";
assert("the other holder is unchanged", b.v, "same");

request.br_concat = "r";
request.br_concat &= "q";
assert("request scope member", request.br_concat, "rq");

suiteEnd();
</cfscript>
