<cfscript>
function build() { var q = queryNew("id,name,val,x,y", "integer,varchar,varchar,varchar,varchar"); for (var i=1;i<=2000;i++){ queryAddRow(q, {id=i,name="name#i#",val="value-#i#-xxxxxxxxxxxxxxxx",x="x#i#",y="y#i#"}); } return q; }
function passThrough(required query q) { return arguments.q; }
function intoStruct(required query q) { var s = { data=arguments.q, n=arguments.q.recordCount }; return s.data; }
function viaCache(required query q) { cachePut("qref", arguments.q); return cacheGet("qref"); }
function intoArray(required query q) { var a = []; arrayAppend(a, arguments.q); return a[1]; }
function dup(required query q) { return duplicate(arguments.q); }
q = build();
for (i=1;i<=200;i++) { a = passThrough(q); b = intoStruct(q); c = viaCache(q); d = intoArray(q); }
e = dup(q);
writeOutput("rows=" & q.recordCount & chr(10));
</cfscript>
