component {
    // Stands in for WireBox's objectBuilder: a method member-extracted from HERE
    // and injected into another component. When invoked as a member of that other
    // component it must bind to the RECEIVER's this/variables (call-site binding),
    // not to Origin. getFunctionCalledName() must report the injected name.
    this.marker = "ORIGIN";
    function provide(){
        return this.marker & ":" & getFunctionCalledName();
    }
}
