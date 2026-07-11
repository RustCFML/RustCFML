component accessors=true {
    property name="ref"   type="any";
    property name="label" type="string" default="deflabel";
    // A genuine PUBLIC this member (explicit assignment, NOT an accessor write).
    function init() { this.publicFlag = "iampublic"; return this; }
}
