component {
    // Base method — inherited by DebugFooterKid. Exercised by the debug-footer
    // "pages" regression test (a CFC method call must fire a template hit).
    public string function baseHello() {
        return "hello from base";
    }
}
