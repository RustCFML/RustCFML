<cfscript>
// ns per operation for struct/array primitives, all inside a function (realistic locals)
function t(label, fn, n) {
	fn(100);
	var t0 = getTickCount("nano"); fn(n); var ns = (getTickCount("nano") - t0) / n;
	out &= left(label & repeatString(" ", 34), 34) & numberFormat(ns, "0") & chr(10);
}
out = "";
N = url.n ?: 200000;
t("empty struct literal {}", function(n){ for (var i=1;i<=n;i++) { var s = {}; } }, N);
t("struct literal 5 keys", function(n){ for (var i=1;i<=n;i++) { var s = {a=1,b="x",c=true,d=2.5,e=i}; } }, N);
t("nested literal {a={b=[1,2]}}", function(n){ for (var i=1;i<=n;i++) { var s = {a={b=[1,2]},c=i}; } }, N);
t("empty array literal []", function(n){ for (var i=1;i<=n;i++) { var a = []; } }, N);
t("array literal 5", function(n){ for (var i=1;i<=n;i++) { var a = [1,"x",true,2.5,i]; } }, N);
t("struct set 10 keys (s.k=)", function(n){ for (var i=1;i<=n/10;i++) { var s = {}; s.k1=1;s.k2=2;s.k3=3;s.k4=4;s.k5=5;s.k6=6;s.k7=7;s.k8=8;s.k9=9;s.k10=10; } }, N);
t("struct set bracket s[k]=", function(n){ var s = {}; for (var i=1;i<=n;i++) { s["k" & (i mod 50)] = i; } }, N);
t("struct get dot s.k", function(n){ var s = {alpha=1,beta=2,gamma=3,delta=4}; var x=0; for (var i=1;i<=n;i++) { x = s.gamma; } }, N);
t("struct get nested s.a.b.c", function(n){ var s = {a={b={c=1}}}; var x=0; for (var i=1;i<=n;i++) { x = s.a.b.c; } }, N);
t("structKeyExists", function(n){ var s = {alpha=1,beta=2,gamma=3}; var x=0; for (var i=1;i<=n;i++) { x = structKeyExists(s, "beta"); } }, N);
t("s.keyExists() member", function(n){ var s = {alpha=1,beta=2,gamma=3}; var x=0; for (var i=1;i<=n;i++) { x = s.keyExists("beta"); } }, N);
t("isNull(s.k ?: nullValue()) elvis", function(n){ var s = {alpha=1}; var x=0; for (var i=1;i<=n;i++) { x = s.beta ?: 0; } }, N);
t("arrayAppend x10", function(n){ for (var i=1;i<=n/10;i++) { var a = []; for (var j=1;j<=10;j++) { arrayAppend(a, j); } } }, N);
t("a.append() member x10", function(n){ for (var i=1;i<=n/10;i++) { var a = []; for (var j=1;j<=10;j++) { a.append(j); } } }, N);
t("array index read a[i]", function(n){ var a = [1,2,3,4,5,6,7,8,9,10]; var x=0; for (var i=1;i<=n;i++) { x = a[(i mod 10)+1]; } }, N);
t("for-in struct 10 keys /key", function(n){ var s={k1=1,k2=2,k3=3,k4=4,k5=5,k6=6,k7=7,k8=8,k9=9,k10=10}; var x=0; for (var i=1;i<=n/10;i++) { for (var k in s) { x = s[k]; } } }, N);
t("for-in array 10 /item", function(n){ var a=[1,2,3,4,5,6,7,8,9,10]; var x=0; for (var i=1;i<=n/10;i++) { for (var v in a) { x = v; } } }, N);
t("structCopy 10 keys", function(n){ var s={k1=1,k2=2,k3=3,k4=4,k5=5,k6=6,k7=7,k8=8,k9=9,k10=10}; for (var i=1;i<=n;i++) { var c = structCopy(s); } }, N);
t("duplicate nested", function(n){ var s={a={b=[1,2,3]},c="x",d={e=1}}; for (var i=1;i<=n;i++) { var c = duplicate(s); } }, N);
t("structAppend 5 into 5", function(n){ var src={a=1,b=2,c=3,d=4,e=5}; for (var i=1;i<=n;i++) { var d={f=1,g=2,h=3,j=4,k=5}; structAppend(d, src); } }, N);
t("structKeyList 10", function(n){ var s={k1=1,k2=2,k3=3,k4=4,k5=5,k6=6,k7=7,k8=8,k9=9,k10=10}; for (var i=1;i<=n;i++) { var l = structKeyList(s); } }, N);
t("arrayLen", function(n){ var a=[1,2,3,4,5]; var x=0; for (var i=1;i<=n;i++) { x = arrayLen(a); } }, N);
t("a.map(closure) 10", function(n){ var a=[1,2,3,4,5,6,7,8,9,10]; for (var i=1;i<=n/10;i++) { var b = a.map(function(v){ return v*2; }); } }, N);
t("s.each(closure) 10", function(n){ var s={k1=1,k2=2,k3=3,k4=4,k5=5,k6=6,k7=7,k8=8,k9=9,k10=10}; var x=0; for (var i=1;i<=n/10;i++) { s.each(function(k,v){ x = v; }); } }, N);
t("pass struct to fn & return", function(n){ var s={a=1}; var f = function(x){ return x; }; for (var i=1;i<=n;i++) { var r = f(s); } }, N);
t("empty loop baseline", function(n){ for (var i=1;i<=n;i++) { } }, N);
writeOutput(out);
</cfscript>
