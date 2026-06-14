// Defines a method that reaches its package-sibling by BARE name. The bare
// name must resolve relative to THIS component's directory (oop/inh/) — the
// directory where the CreateObject literal is lexically defined — regardless
// of which subclass instance the method runs on.
component {
    public string function viaCreate() {
        try {
            return CreateObject("component", "InhSibling").hi();
        } catch (any e) {
            return "ERR:" & e.message;
        }
    }
}
