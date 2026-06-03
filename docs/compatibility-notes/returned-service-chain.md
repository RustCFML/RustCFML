# Returned Service Chain Receiver Preservation

Moopa resolves table services dynamically and immediately calls methods on the
returned service:

```cfml
application.lib.db.getService("moo_profile").login(...)
```

Lucee-compatible behavior is that `getService()` runs on the factory component,
`login()` runs on the returned service component, and the original factory
receiver remains unchanged after the chained call.

The added CFML test captures that behavior without prescribing an implementation
strategy.
