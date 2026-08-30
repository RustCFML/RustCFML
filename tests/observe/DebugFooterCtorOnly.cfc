component {
    // Constructed but never method-called, so the ONLY thing that can produce a
    // pages row for this file is the construction itself. Guards the v0.645 fix:
    // component METHODS opened a timed frame but `new X()` did not, so every
    // microsecond of object building was invisible in the footer and landed in
    // the top-level page's residual row instead.
    public any function init() {
        return this;
    }
    public string function neverCalled() {
        return "unused";
    }
}
