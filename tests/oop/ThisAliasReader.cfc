/**
 * Reads the `this` key off a foreign component's `variables` scope — models
 * Wheels `Plugins.$initializeMixins(variablesScope)` reading
 * `GetMetadata(variablesScope.this)`. See ThisAliasProbe.cfc.
 */
component {
    struct function read(required struct variablesScope) {
        var r = {};
        r.hasThisKey = structKeyExists(arguments.variablesScope, "this");
        var md = getMetadata(arguments.variablesScope.this);
        r.isObj = isObject(arguments.variablesScope.this);
        r.hasName = structKeyExists(md, "name");
        r.hasFullname = structKeyExists(md, "fullname");
        r.name = r.hasName ? md.name : "";
        return r;
    }
}
