<cfscript>
// Target page for <cfflush> tests, called via cfhttp from test_cfflush.cfm.
//
// Every flush case lives here rather than in the suite itself because a flush
// COMMITS the response: doing it in the runner would freeze the runner's own
// headers and make every later cfheader/cflocation/cfcontent test fail.

param name="url.test" default="";

switch (url.test) {
    // A flush commits the status and headers set BEFORE it, and drops the ones
    // set after (which raise — see the "after-*" cases below).
    case "commit":
        header name="X-Before" value="yes";
        header statuscode="201" statustext="Created";
        writeOutput("first-chunk");
        cfflush();
        writeOutput("|second-chunk");
        break;

    // Lucee's getRootOut() is the BASE writer, so a flush inside a capture
    // pushes what preceded the capture and leaves the capture itself intact.
    // (BoxLang's force-flush drags captured buffers out too; we follow Lucee.)
    case "savecontent":
        writeOutput("page-text");
        savecontent variable="cap" {
            writeOutput("A");
            cfflush();
            writeOutput("B");
        }
        writeOutput("|cap=[" & cap & "]");
        break;

    // interval="N" turns on auto-flushing once the buffer passes N bytes. The
    // whole body must still arrive, in order.
    case "interval":
        cfflush(interval=16);
        for (i = 1; i <= 5; i++) {
            writeOutput("segment-" & i & ";");
        }
        break;

    // A second flush, and an abort after a flush, are both fine on Lucee.
    case "reflush":
        writeOutput("one");
        cfflush();
        cfflush();
        writeOutput("|two");
        cfflush();
        break;

    // cfcookie after a flush does NOT raise on Lucee (it is simply ineffective).
    case "after-cookie":
        writeOutput("x");
        cfflush();
        try {
            cookie name="cfflushtest" value="v";
            writeOutput("|no-throw");
        } catch (any e) {
            writeOutput("|threw:" & e.message);
        }
        break;

    case "after-header":
        writeOutput("x");
        cfflush();
        try {
            header name="X-After" value="nope";
            writeOutput("|no-throw");
        } catch (any e) {
            writeOutput("|threw:" & e.message);
        }
        break;

    case "after-content":
        writeOutput("x");
        cfflush();
        try {
            content type="text/plain";
            writeOutput("|no-throw");
        } catch (any e) {
            writeOutput("|threw:" & e.message);
        }
        break;

    case "after-location":
        writeOutput("x");
        cfflush();
        try {
            location url="/nowhere.cfm";
            writeOutput("|no-throw");
        } catch (any e) {
            writeOutput("|threw:" & e.message);
        }
        break;

    case "after-htmlhead":
        writeOutput("x");
        cfflush();
        try {
            htmlhead text="<!--injected-->";
            writeOutput("|no-throw");
        } catch (any e) {
            writeOutput("|threw:" & e.message);
        }
        break;

    // A negative or zero interval is not an error on Lucee — setBufferConfig
    // stores it and the buffer-length check is then always true, i.e. flush on
    // every write. The body must still arrive intact.
    case "neg-interval":
        try {
            cfflush(interval=-5);
            writeOutput("neg-ok");
        } catch (any e) {
            writeOutput("neg-threw:" & e.message);
        }
        try {
            cfflush(interval=0);
            writeOutput("|zero-ok");
        } catch (any e) {
            writeOutput("|zero-threw:" & e.message);
        }
        writeOutput("|body-intact");
        break;

    // isFlushed() tracks exactly the commit flag that decides whether
    // cfheader/cfcontent/cflocation still work.
    case "isflushed":
        writeOutput("before=" & isFlushed());
        cfflush();
        writeOutput("|after=" & isFlushed());
        cfflush();
        writeOutput("|again=" & isFlushed());
        break;

    // A flush inside a cfthread body must not disturb the request's own output:
    // the thread writes to its own buffer and has no client of its own.
    case "thread":
        writeOutput("before");
        thread name="flushThread" { cfflush(); }
        thread action="join" name="flushThread" timeout="5";
        writeOutput("|after");
        cfflush();
        break;

    default:
        writeOutput("unknown-test");
}
</cfscript>
