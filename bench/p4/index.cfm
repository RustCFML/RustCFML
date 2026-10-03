<cfscript>
n = 2000;
state = new State( [ new Listener(), new Listener(), new Listener() ] );
svc = new Svc( state ); svc.setCtx( new Ctx() );
d = { x=1 };
t0 = getTickCount("nano"); for (i=1;i<=n;i++) svc.processState( "s1", d ); t1 = getTickCount("nano");
for (i=1;i<=n;i++) svc.processStateNoBuffer( "s1", d ); t2 = getTickCount("nano");
for (i=1;i<=n;i++) svc.processStateDirect( "s1", d ); t3 = getTickCount("nano");
writeOutput("full=" & numberFormat((t1-t0)/1000/n,"0.0") & "us noBuffer=" & numberFormat((t2-t1)/1000/n,"0.0") & "us direct=" & numberFormat((t3-t2)/1000/n,"0.0") & "us" & chr(10));
</cfscript>
