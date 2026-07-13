component {
    this.name = "rustcfml-app-shared-vars-test";

    // Regression (Masa CMS boot): a method called DURING the Application.cfc
    // pseudo-constructor that writes `variables.x` must be visible to a sibling
    // method it calls. Masa's applicationSettings.cfm does exactly this:
    //   initINI() -> variables.ini = structNew() -> setINISection() reads it.
    // The app-component construction path must build a SHARED variables scope
    // just like a normal `new Foo()` does.
    initINI();

    // Regression (Masa CMS boot): a missing/unreadable DYNAMIC include is a
    // CATCHABLE `missingInclude` error, not a hard abort. Masa's
    //   try { include "#context#/plugins/mappings.cfm" } catch(any e){...}
    // depends on it. Use an interpolated path to force the IncludeDynamic op.
    variables.missingCaught = "no";
    missingPath = "does_not_exist_" & hash("masa") & ".cfm";
    try {
        include "#missingPath#";
    } catch (any e) {
        variables.missingCaught = (e.type eq "missingInclude") ? "yes" : ("othertype:" & e.type);
    }

    function initINI() output=false {
        variables.ini = structNew();
        setINISection("settings");
        variables.ini["settings"]["mode"] = "prod";
    }

    void function setINISection(required string section) output=false {
        if (!structKeyExists(variables.ini, arguments.section)) {
            variables.ini[arguments.section] = structNew();
        }
    }

    function onRequest(targetPage) {
        var iniOk = (structKeyExists(variables, "ini")
            && structKeyExists(variables.ini, "settings")
            && structKeyExists(variables.ini.settings, "mode")
            && variables.ini.settings.mode == "prod") ? "ok" : "MISSING";
        writeOutput("ini=" & iniOk & ";missing=" & variables.missingCaught);
    }
}
