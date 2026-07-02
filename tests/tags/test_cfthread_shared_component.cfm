<cfscript>
suiteBegin("cfthread: shared component attribute + thread-safe queue (GH ##234)");

// 1) Script-form `thread ... svc="#comp#" { ... }` must expose the custom
//    attribute as the `attributes` scope inside the body (the tag form already
//    did; the script form dropped every attribute but name/action). And a
//    component handed in that way is a SHARED reference, so method calls inside
//    the thread are visible from the parent after join.
c = new tags.thr234.Counter234();
for ( i = 1; i <= 10; i++ ) {
    thread name="qt#i#" action="run" svc="#c#" n="#i#" {
        for ( j = 1; j <= 5; j++ ) {
            attributes.svc.add( { t: attributes.n, e: j } );
        }
    }
}
thread action="join" name="qt1,qt2,qt3,qt4,qt5,qt6,qt7,qt8,qt9,qt10" timeout="10000";
assert("50 concurrent adds across 10 threads are all visible after join (thread-safe)",
    c.count(), 50);

// 2) Code inside a cfthread body can detect it is running in a thread.
outside = isInThread();
thread name="ctx" action="run" {
    thread.inside = isInThread();
}
thread action="join" name="ctx" timeout="5000";
assertFalse("isInThread() is false on the main thread", outside);
assertTrue("isInThread() is true inside a cfthread body", cfthread.ctx.inside);

suiteEnd();
</cfscript>
