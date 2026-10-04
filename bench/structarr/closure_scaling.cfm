<cfscript>
function t(label, fn, n) { fn(100); var t0 = getTickCount("nano"); fn(n); out &= left(label & repeatString(" ", 40), 40) & numberFormat((getTickCount("nano") - t0) / n, "0") & chr(10); }
out = ""; N = 300000;
t("closure call, parent has 0 extra locals", function(n){  var g = function(x){ return x; }; var r = 0; for (var i=1;i<=n;i++) { r = g(i); } }, N);
t("closure call, parent has 10 extra locals", function(n){ var p0 = 0; var p1 = 1; var p2 = 2; var p3 = 3; var p4 = 4; var p5 = 5; var p6 = 6; var p7 = 7; var p8 = 8; var p9 = 9; var g = function(x){ return x; }; var r = 0; for (var i=1;i<=n;i++) { r = g(i); } }, N);
t("closure call, parent has 40 extra locals", function(n){ var p0 = 0; var p1 = 1; var p2 = 2; var p3 = 3; var p4 = 4; var p5 = 5; var p6 = 6; var p7 = 7; var p8 = 8; var p9 = 9; var p10 = 10; var p11 = 11; var p12 = 12; var p13 = 13; var p14 = 14; var p15 = 15; var p16 = 16; var p17 = 17; var p18 = 18; var p19 = 19; var p20 = 20; var p21 = 21; var p22 = 22; var p23 = 23; var p24 = 24; var p25 = 25; var p26 = 26; var p27 = 27; var p28 = 28; var p29 = 29; var p30 = 30; var p31 = 31; var p32 = 32; var p33 = 33; var p34 = 34; var p35 = 35; var p36 = 36; var p37 = 37; var p38 = 38; var p39 = 39; var g = function(x){ return x; }; var r = 0; for (var i=1;i<=n;i++) { r = g(i); } }, N);
t("UDF call (no closure)", function(n){ var r = 0; for (var i=1;i<=n;i++) { r = plainUdf(i); } }, N);
function plainUdf(x) { return x; }
t("arr.map(closure) per element", function(n){ var a=[1,2,3,4,5,6,7,8,9,10]; for (var i=1;i<=n/10;i++) { var b = a.map(function(v){ return v; }); } }, N);
writeOutput(out);
</cfscript>