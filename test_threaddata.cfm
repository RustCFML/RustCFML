<cfscript>
// Simple test for threadData()
thread name="test1" {
    sleep(10);
}
sleep(20);
var data = threadData();
writeDump(data);
</cfscript>
