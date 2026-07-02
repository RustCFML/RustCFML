<cfscript>
function t(v, names){
  for(n in names) writeoutput(n & "=" & isInstanceOf(v,n) & " ");
  writeoutput(chr(10));
}
writeoutput("ARR: "); t([1], ["Array","java.util.List","java.util.Collection","java.lang.Iterable","lucee.runtime.type.ArrayImpl","lucee.runtime.type.Array"]);
writeoutput("ST: "); t({a=1}, ["Struct","java.util.Map","lucee.runtime.type.StructImpl","lucee.runtime.type.Struct"]);
writeoutput("STR: "); t("x", ["String","java.lang.String","java.lang.CharSequence","java.lang.Comparable","simple"]);
writeoutput("BOOL: "); t(true, ["Boolean","java.lang.Boolean","boolean"]);
writeoutput("INT: "); t(42, ["numeric","Numeric","java.lang.Double","java.lang.Number","java.lang.Integer"]);
writeoutput("QRY: "); q=queryNew("a"); t(q, ["Query","java.util.List","lucee.runtime.type.QueryImpl","lucee.runtime.type.Query"]);
</cfscript>
