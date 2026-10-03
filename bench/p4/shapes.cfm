<cfscript>
n = 5000; s = new Shapes(); o = new Shapes();
for (i=1;i<=n;i++) { s.zero(); s.five(1,2,3,4,5); s.fiveTyped(1,2,3,4,5); s.fiveDefaults(1); s.withLocal(); s.dyn(o); s.dynNamed(o); s.passColl(1,2,3,4,5); s.passCollExtra(1,2,3); s.callNamed(); s.callPos(); s.argsDot(1,2); s.mCallNamed(o); s.mPassColl(o,1,2,3,4,5); s.mCallPos(o); }
writeOutput("ok" & chr(10));
</cfscript>
