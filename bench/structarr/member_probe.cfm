<cfscript>
function show(v) { if (isArray(v)) return "array(" & arrayLen(v) & ")"; if (isStruct(v)) return "struct(" & structCount(v) & ")"; if (isBoolean(v) && !isNumeric(v)) return "bool:" & v; return "value:" & v; }
a = [1]; r1 = a.append(2);
s = { b = 1, n = javaCast("null", "") };
o = [];
o.append( show(r1) & " same=" & (r1 === a ? "Y" : "N") & " a=" & arrayLen(a) );
o.append( "a.len=" & show(a.len()) );
o.append( "[].isEmpty=" & show([].isEmpty()) & " a.isEmpty=" & show(a.isEmpty()) );
o.append( "s.keyExists(b)=" & show(s.keyExists("b")) & " (B)=" & show(s.keyExists("B")) & " (zz)=" & show(s.keyExists("zz")) & " (null n)=" & show(s.keyExists("n")) );
o.append( "s.len=" & show(s.len()) & " {}.isEmpty=" & show({}.isEmpty()) & " s.isEmpty=" & show(s.isEmpty()) );
u = { append = function(x) { return "udf:" & x; }, len = function() { return "udfLen"; } };
o.append( "struct with udf append=" & u.append(5) & " len=" & u.len() );
m = [[1],[2]]; m[1].append(9); o.append("nested element append: " & serializeJSON(m));
writeOutput(arrayToList(o, chr(10)));
</cfscript>
