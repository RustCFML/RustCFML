component {
    function make() {
        // Unqualified new — must resolve relative to THIS component's package (tests.oop.pkg229)
        return new Widget229();
    }
}
