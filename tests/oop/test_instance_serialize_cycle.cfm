<cfscript>
// GH ##178 (flyweight re-opening): a self-referential or mutually-referential
// COMPONENT graph must not abort the engine when serialized/dumped. The #178
// cycle-guard (v0.219.0) covered Struct/Array/marker-component via backing-Arc
// pointers, but the flyweight `Instance` serialize/dump arms bypassed it — and a
// flyweight instance materialises a FRESH data struct each call, so the
// downstream struct-ptr guard could never fire. Any cyclic instance graph then
// recursed until the native stack overflowed (SIGABRT — uncatchable, killed the
// serve worker). This surfaced as a boot crash running Preside's TestBox suite
// under the v0.519.0 default-on flyweight flip: MockBox mocks form the classic
// `mock -> mockGenerator -> mock` cycle that normalizeArguments serialises.
// The guard now keys on the instance's own Arc identity. Runs on BOTH the marker
// and flyweight builds (marker already guarded via the struct path).
suiteBegin("Instance serialize/dump cycle guard (GH ##178 flyweight)");

// --- self reference: a.self = a ---
a = new oop.CycleNode();
a.self = a;
js = serializeJSON(a);
assertTrue("serializeJSON of a self-referential instance completes (no overflow)", len(js) GT 0);
assertTrue("serializeJSON self-ref keeps the member key", findNoCase("self", js) GT 0);
// The cycle is broken (bounded output), not infinitely nested.
assertTrue("serializeJSON self-ref output is bounded", len(js) LT 200);

// --- mutual reference: b.other = c, c.other = b ---
b = new oop.CycleNode();
c = new oop.CycleNode();
b.other = c;
c.other = b;
jsm = serializeJSON(b);
assertTrue("serializeJSON of mutually-referential instances completes", len(jsm) GT 0);
assertTrue("serializeJSON mutual keeps the member key", findNoCase("other", jsm) GT 0);
assertTrue("serializeJSON mutual output is bounded", len(jsm) LT 200);

// --- Serialize() (CFML-literal) path ---
ser = serialize(a);
assertTrue("Serialize() of a self-referential instance completes", len(ser) GT 0);
assertTrue("Serialize() self-ref output is bounded", len(ser) LT 200);

// --- writeDump path (both the recursion + the recursion marker) ---
savecontent variable="dmp" { writeDump(a); }
assertTrue("writeDump of a self-referential instance completes", len(dmp) GT 0);
assertTrue("writeDump marks the recursion instead of overflowing", findNoCase("recursive", dmp) GT 0);

// Non-cyclic instances still serialize their data normally (guard is inert).
plain = new oop.CycleNode();
plain.name = "ok";
plain.n = 42;
pjs = serializeJSON(plain);
assertTrue("non-cyclic instance still serializes its data", findNoCase("ok", pjs) GT 0 AND findNoCase("42", pjs) GT 0);

suiteEnd();
</cfscript>
