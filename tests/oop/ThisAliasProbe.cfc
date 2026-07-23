/**
 * Regression fixture for the live `variables.this` alias (Lucee/ACF parity).
 * A component's private scope exposes a `this` key that resolves to the whole
 * component, so `getMetadata(variables.this).fullname` and
 * `isObject(variables.this)` recognise it. This is exactly what Wheels'
 * `Plugins.$initializeMixins` reads at boot (`GetMetadata(variablesScope.this).fullname`)
 * — under the flyweight (component-instance) build the alias used to resolve to the
 * bare public data map (no name/fullname), 500ing the Wheels boot.
 */
component {
    function init() {
        return this;
    }

    function greet() {
        return "hi";
    }

    // Introspect our own `variables.this` (the failing Wheels read path).
    struct function introspectSelf() {
        var r = {};
        r.hasThisKey = structKeyExists(variables, "this");
        r.isObj = isObject(variables.this);
        var md = getMetadata(variables.this);
        r.hasName = structKeyExists(md, "name");
        r.hasFullname = structKeyExists(md, "fullname");
        r.name = r.hasName ? md.name : "";
        r.fullname = r.hasFullname ? md.fullname : "";
        return r;
    }

    // Cross-object: hand our `variables` scope to another component and let it read
    // `variablesScope.this` — the exact Wheels `$initializeMixins(variables)` shape.
    struct function introspectViaReader() {
        return new oop.ThisAliasReader().read(variables);
    }

    // Mixin injection via `variables.this` (the exact Wheels Plugins.cfc:821 pattern:
    // `StructAppend(variablesScope.this, mixins, true)`). The appended method must
    // become callable on the public object — this exercises that switching
    // `variables.this` to resolve as the whole Instance still routes writes to the
    // public scope.
    function injectMixin() {
        structAppend(variables.this, { injectedMixin: function() { return "mixed-in"; } }, true);
    }
}
