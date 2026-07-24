component extends="DebugFooterBase" {
    // Child that extends DebugFooterBase. Instantiating + calling a method here
    // must produce a "pages" row for DebugFooterKid.cfc in the debug footer —
    // the v0.519 flyweight flip regressed this to zero CFC rows (only .cfm
    // includes + Application.cfc lifecycle showed). See test_debug_footer.cfm.
    public string function kidRun() {
        return baseHello() & " via kid";
    }
}
