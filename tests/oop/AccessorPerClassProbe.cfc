component accessors=true {
    property name="label" type="string" default="L";
    property name="custom" type="string" default="C";
    public string function getCustom() { return "explicit"; }
    public struct function probe() {
        return {
            varHasGetter: structKeyExists(variables, "getLabel"),
            varHasSetter: structKeyExists(variables, "setLabel"),
            thisHasGetter: structKeyExists(this, "getLabel"),
            bareGetter: getLabel(),
            explicitWins: getCustom(),
            viaVariables: variables.getCustom()
        };
    }
}
