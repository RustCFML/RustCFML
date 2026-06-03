<cfscript>
suiteBegin("Returned service chain");

factory = createObject("component", "oop.ChainFactory").init();

assert(
    "method call works on CFC returned from factory method",
    factory.getService("moo_profile").login(profile_id = "p1", stay_logged_in = false),
    "service:p1:false"
);

assert(
    "factory receiver is not overwritten by chained returned-service call",
    factory.kind(),
    "factory"
);

assert(
    "factory can still create another service after chained call",
    factory.getService("moo_profile").login(profile_id = "p2", stay_logged_in = false),
    "service:p2:false"
);

application.lib = {};
application.lib.db = createObject("component", "oop.ChainFactory").init();

assert(
    "application-scoped factory supports Moopa-style service chain",
    application.lib.db.getService("moo_profile").login(profile_id = "p3", stay_logged_in = false),
    "service:p3:false"
);

assert(
    "application-scoped factory remains the factory after service chain",
    application.lib.db.kind(),
    "factory"
);

suiteEnd();
</cfscript>
