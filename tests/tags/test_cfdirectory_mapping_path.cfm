<cfscript>
suiteBegin("cfdirectory mapping paths");
</cfscript>
<cfdirectory action="list"
    directory="/oop/native_cfcs"
    name="mappedCfcFiles"
    filter="*.cfc">
<cfscript>
assert("mapped directory resolves via Application.cfc mapping", mappedCfcFiles.recordCount, 3);

loader = createObject("component", "oop.CfdirectoryLoader");
assert("cfdirectory unscoped name inside component method", loader.loadRoutes(), "3|name,directory,size,type,dateLastModified,attributes,mode");
suiteEnd();
</cfscript>
