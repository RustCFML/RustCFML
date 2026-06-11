<cfscript>
// cfdirectory recurse=true must traverse symlinked directories (Lucee parity).
//
// Lucee's recursive directory listing follows directory symlinks; RustCFML's
// does not, so files reachable only through a symlinked directory silently
// vanish from the listing. This silently breaks apps that assemble route or
// component registries from a recursive scan over a tree that contains
// symlinks.
//
// Creating a symlink portably from CFML needs `ln -s` via cfexecute, so this
// test is POSIX-only and self-skips when the link cannot be created.

suiteBegin("cfdirectory recurse=true traverses symlinked directories");

lnskip = false;
base = getTempDirectory() & "/rcfml_symlink_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));
realDir = base & "/real_dir";
linkDir = base & "/linked_dir";

try {
    directoryCreate(realDir, true);
    fileWrite(realDir & "/Probe.cfc", "component {}");
} catch (any e) {
    lnskip = true;
    writeOutput("  (skipped — cannot create fixture dir: " & e.message & ")" & chr(10));
}
</cfscript>

<cfif NOT lnskip>
    <cftry>
        <cfexecute name="/bin/ln" arguments="-s #realDir# #linkDir#" timeout="10" />
        <cfcatch type="any">
            <cfset lnskip = true />
            <cfoutput>  (skipped — cannot create symlink: #cfcatch.message#)#chr(10)#</cfoutput>
        </cfcatch>
    </cftry>
</cfif>

<cfif NOT lnskip>
    <cfif NOT directoryExists(linkDir)>
        <cfset lnskip = true />
        <cfoutput>  (skipped — symlink was not created)#chr(10)#</cfoutput>
    </cfif>
</cfif>

<cfif NOT lnskip>
    <!--- control: listing the symlinked directory AS the root works today --->
    <cfdirectory action="list" directory="#linkDir#" name="qDirect" filter="*.cfc" />
    <cfscript>
    assert("listing through a symlinked root finds the file (control)",
        qDirect.recordCount, 1);
    </cfscript>

    <!--- control: non-recursive listing of the parent sees both entries --->
    <cfdirectory action="list" directory="#base#" name="qTop" />
    <cfscript>
    assert("non-recursive parent listing sees both dirs (control)",
        qTop.recordCount, 2);
    </cfscript>

    <!--- gap: recursive listing must traverse INTO the symlinked directory.
          Lucee finds Probe.cfc twice (real_dir + linked_dir); an engine that
          does not follow directory symlinks finds it once. --->
    <cfdirectory action="list" directory="#base#" name="qRec" recurse="true" filter="*.cfc" />
    <cfscript>
    assert("recursive listing traverses the symlinked directory",
        qRec.recordCount, 2);
    </cfscript>
</cfif>

<cfscript>
try {
    if (directoryExists(base)) {
        directoryDelete(base, true);
    }
} catch (any e) {}

suiteEnd();
</cfscript>
