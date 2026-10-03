<cfscript>
suiteBegin("Core: engine exception types match Lucee (GH ##451)");

// Measured against Lucee 7.1.0.204.
function _typeOf(f) {
	try { f(); return "no error"; } catch (any e) { return e.type; }
}
assert("string + number is expression", _typeOf(function() { x = "abc" + 1; }), "expression");
assert("string * string is expression", _typeOf(function() { x = "a" * "b"; }), "expression");
assert("struct + number is expression", _typeOf(function() { x = {} + 1; }), "expression");
assert("missing component is expression", _typeOf(function() { x = createObject("component", "NoSuchCfc451"); }), "expression");
assert("bad JSON is expression", _typeOf(function() { x = deserializeJSON("{bad"); }), "expression");
assert("1 / 0 is ArithmeticException", _typeOf(function() { x = 1 / 0; }), "java.lang.ArithmeticException");
assert("mod 0 is ArithmeticException", _typeOf(function() { x = 5 mod 0; }), "java.lang.ArithmeticException");
assert("integer division by 0 is ArithmeticException", _typeOf(function() { x = 5 \ 0; }), "java.lang.ArithmeticException");

// e.type is compared as a string too, so the casing matters.
try { throw(message = "m"); } catch (any e) { _t = e.type; }
assertTrue("throw() defaults to lowercase application", compare(_t, "application") == 0);
try { throw "bare"; } catch (any e) { _t = e.type; }
assertTrue("bare throw defaults to lowercase application", compare(_t, "application") == 0);
_t = "";
try { x = "abc" + 1; } catch (any e) { _t = e.type; }
assertTrue("e.type of a failed cast is lowercase expression", compare(_t, "expression") == 0);

// catch(expression) must catch a failed cast in the SAME frame — the try used
// to be skipped for errors raised by arithmetic ops.
_caught = "none";
try { x = "abc" + 1; } catch (expression e) { _caught = "expression"; } catch (any e) { _caught = "any"; }
assert("catch(expression) catches string + number at page level", _caught, "expression");
_caught = "none";
cstr = "abc";
try { cstr -= 1; } catch (expression e) { _caught = "expression"; } catch (any e) { _caught = "any"; }
assert("catch(expression) catches a compound assignment", _caught, "expression");
function _inFrame() {
	try { var y = "abc" * 2; return "no"; } catch (expression e) { return "caught"; }
}
assert("catch(expression) in the same function frame", _inFrame(), "caught");
_caught = "none";
try { x = 1 / 0; } catch (expression e) { _caught = "expression"; } catch (any e) { _caught = "any"; }
assert("catch(expression) does not catch division by zero", _caught, "any");
_caught = "none";
try { x = 7 mod 0; } catch (java.lang.ArithmeticException e) { _caught = "arith"; } catch (any e) { _caught = "any"; }
assert("catch(java.lang.ArithmeticException) catches mod 0", _caught, "arith");
_after = "";
try { x = "abc" + 1; } catch (any e) { _after = "handled"; }
_after &= "|continued";
assert("execution continues after the handler", _after, "handled|continued");

// The fused compound ops (`+=`, `*=`, `++`, `--`) coerce like the binary
// operators. They used to replace a non-number with the bare step.
cnum = "5"; cnum += 1;
assert("numeric string += 1", cnum, 6);
cnum = true; cnum += 1;
assert("boolean += 1", cnum, 2);
cnum = "2.5"; cnum++;
assert("numeric string ++", cnum, 3.5);
cnum = "4"; cnum--;
assert("numeric string --", cnum, 3);
cnum = "3"; cnum *= 3;
assert("numeric string *= 3", cnum, 9);
function _compound() { var z = "5"; z += 2; z *= 3; return z; }
assert("numeric string compound ops on a local", _compound(), 21);
_caught = "none";
cstr = "abc";
try { cstr++; } catch (expression e) { _caught = "expression"; }
assert("++ on a non-numeric string throws expression", _caught, "expression");

suiteEnd();
</cfscript>
