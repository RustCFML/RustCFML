/**
 * GH #285 fixture: shallow struct-copy operations sourced from a component's
 * `variables` scope must keep function members (Lucee treats functions as
 * ordinary struct values here). Regressed v0.507.0 (flyweight method table not
 * enumerated by the Struct-Struct copy path).
 */
component {

    public string function pluralize() { return "books"; }
    private string function secret()    { return "sh"; }

    // StructAppend(dest, variables) — Wheels' plugin snapshot pattern.
    public struct function viaStructAppend() {
        var c = {};
        StructAppend(c, variables);
        return {
            pluralize = StructKeyExists(c, "pluralize"),  // public method
            secret    = StructKeyExists(c, "secret"),      // private method
            this_leak = StructKeyExists(c, "this"),
            super_leak= StructKeyExists(c, "super")
        };
    }

    // StructCopy(variables)
    public struct function viaStructCopy() {
        var c = StructCopy(variables);
        return {
            pluralize = StructKeyExists(c, "pluralize"),
            secret    = StructKeyExists(c, "secret")
        };
    }

    // member-form dest.append(variables)
    public struct function viaMemberAppend() {
        var c = {};
        c.append(variables);
        return { pluralize = StructKeyExists(c, "pluralize") };
    }

    // Expose the live variables scope for an external-reference copy.
    public any function exposeVars() { return variables; }

    // GH #285 (secondary): a member CALL of a missing key on a plain struct must
    // throw (like the rvalue read does), NOT fall back to resolving the bare
    // function name in the ambient scope. Inside a method itself named
    // `pluralize`, that fallback resolved to THIS method and recursed to the
    // depth guard. `core` is a fully markerless plain struct.
    public string function callsMissingMemberOnPlainStruct() {
        var core = {};
        return core.pluralize("x");   // must throw "Variable 'pluralize' is undefined"
    }
}
