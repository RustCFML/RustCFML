<cfscript>
suiteBegin("A method enumerating its component's variables scope");

// Lucee 7.1 lists THIS (last) among a component's variables keys and never
// any engine bookkeeping such as the static scope.
o = new VariablesScopeKeys();
expected = "copyKeys,eachKeys,forInKeys,jsonHasStatic,keyArray,keyCount,keyList,lastKey,priv,staticStillWorks,this,thisFromLoop";
assert("structKeyList(variables) lists this, not __static", o.keyList(), expected);
assert("structKeyArray(variables) agrees", o.keyArray(), expected);
assert("structCount(variables) agrees", o.keyCount(), listLen(expected));
assert("for-in over variables agrees", o.forInKeys(), expected);
assert("this is enumerated last", o.lastKey(), "this");
assert("the enumerated this is the component's public scope", o.thisFromLoop(), 1);
assertFalse("structEach(variables) skips __static", listFindNoCase(o.eachKeys(), "__static") > 0);
assertFalse("structCopy(variables) has no __static", listFindNoCase(o.copyKeys(), "__static") > 0);
assertFalse("serializeJSON(variables) has no __static", o.jsonHasStatic());
assert("the static scope still works", o.staticStillWorks(), 3);

suiteEnd();
</cfscript>
