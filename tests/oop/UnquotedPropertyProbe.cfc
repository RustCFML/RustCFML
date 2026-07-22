/**
 * Fixture for the UNQUOTED `property` attribute syntax
 * (`property name = colour getters = true setters = true type = string;`).
 * Lucee/ACF accept bare (unquoted) scalar values on property attributes; FW/1's
 * DI stubs are written this way. RustCFML previously required QUOTED values to
 * detect the key-value property form, so the unquoted shape was mis-parsed by the
 * positional parser (the leading `name` token became the property name), leaving
 * getMetadata().properties wrong and NO implicit accessors generated —
 * `getColour()` etc. threw "has no function". See
 * tests/oop/test_unquoted_property_accessors.cfm.
 */
component accessors = true {
    property name = colour getters = true setters = true type = string;
    property name = size   getters = true setters = true type = numeric;
    // A bare (valueless) annotation and a quoted attr mixed in, to prove the
    // attribute list still parses to completion.
    property name = label type = string required = false;

    public function init() {
        variables.colour = "red";
        variables.size   = 5;
        variables.label  = "L";
        return this;
    }
}
