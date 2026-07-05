component {
    // Reads a data file that sits next to THIS CFC, via a relative path. There is
    // no file of this name at the base template (the test runner), so a correct
    // engine must fall back to the current template's own directory (Lucee parity).
    function read(){ return fileRead("./relprobe.json"); }
    function readViaExpand(){ return fileExists(expandPath("relprobe.json")); }
}
