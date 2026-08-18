component extends="Parent" {
    // Reached from OUTSIDE (read.cfm) — so it has to be public: a private method
    // is invisible to an external caller (GH #330). The private one below is what
    // the inherited `privateInvoker` pulls out as a value, from inside the class.
    public any function publicAction() { return "child-secret"; }

    private any function secretAction() { return "child-secret"; }
}
