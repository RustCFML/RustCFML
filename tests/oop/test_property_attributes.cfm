<cfscript>
suiteBegin("Property Attributes & Injection Metadata");

// Test getComponentMetadata returns property structs with inject attribute
md = getComponentMetadata("oop.InjectableService");
assertNotNull("metadata returned", md);
assertTrue("has properties", structKeyExists(md, "properties"));
assertTrue("properties is array", isArray(md.properties));
assert("property count", arrayLen(md.properties), 4);

// First property: packageService with inject
prop1 = md.properties[1];
assertTrue("prop1 is struct", isStruct(prop1));
assert("prop1 name", prop1.name, "packageService");
assert("prop1 inject", prop1.inject, "PackageService");

// Second property: print with inject
prop2 = md.properties[2];
assert("prop2 name", prop2.name, "print");
assert("prop2 inject", prop2.inject, "print");

// Third property: configService with inject and hint
prop3 = md.properties[3];
assert("prop3 name", prop3.name, "configService");
assert("prop3 inject", prop3.inject, "ConfigService");
assert("prop3 hint", prop3.hint, "Configuration manager");

// Fourth property: greeting with type and default (no inject)
prop4 = md.properties[4];
assert("prop4 name", prop4.name, "greeting");
assert("prop4 type", prop4.type, "string");
assert("prop4 default surfaced in metadata", prop4.default, "Hello");
assertFalse("prop4 has no inject", structKeyExists(prop4, "inject"));

// Test that component instantiation still works
svc = createObject("component", "oop.InjectableService").init();
assert("component method works", svc.getServiceName(), "InjectableService");
assert("default value set", svc.getGreeting(), "Hello");

// Test tag-based <cfproperty> attributes
tmd = getComponentMetadata("oop.TagPropertyComponent");
assertTrue("tag: has properties", structKeyExists(tmd, "properties"));
assertTrue("tag: properties is array", isArray(tmd.properties));
assert("tag: property count", arrayLen(tmd.properties), 3);

tp1 = tmd.properties[1];
assert("tag: prop1 name", tp1.name, "myService");
assert("tag: prop1 inject", tp1.inject, "MyService");

tp2 = tmd.properties[2];
assert("tag: prop2 name", tp2.name, "helper");
assert("tag: prop2 inject", tp2.inject, "HelperService");
assert("tag: prop2 hint", tp2.hint, "A helper");

tp3 = tmd.properties[3];
assert("tag: prop3 name", tp3.name, "title");
assert("tag: prop3 type", tp3.type, "string");

// --- <cfproperty> attribute order must follow SOURCE declaration order ---
// Tag attributes are parsed into a HashMap, whose iteration order is otherwise
// non-deterministic across processes. cfproperty order is preserved into
// component metadata and feeds identity hashes (Preside derives FK constraint
// names from Hash(SerializeJson(property))), so a stable, source-ordered key
// list is required — name first, then the synthesized `type` default ("any",
// Lucee parity — every property carries a type), then attributes as written.
assert("tag: prop2 attr order is source order", structKeyList(tp2), "name,type,inject,hint");

// --- required="false" must be PRESERVED in property metadata (Lucee parity) ---
// Previously `required` was collapsed to a bool field that codegen only emitted
// when true, so `required="false"` was silently dropped. Preside's
// PresideObjectReaderTest compares the full property attribute struct, so the
// missing `required` key failed the match.
rmd = getComponentMetadata("oop.PropRequiredFixture");
rprops = {};
for ( p in rmd.properties ) { rprops[ p.name ] = p; }
assertTrue("numprop has required key", structKeyExists(rprops.numprop, "required"));
assert("numprop required preserved as false", rprops.numprop.required, "false");
assert("numprop keeps minValue", rprops.numprop.minValue, "1");
assert("numprop keeps maxValue", rprops.numprop.maxValue, "10");
assertTrue("reqprop required=true present", rprops.reqprop.required);
assertFalse("plainprop has no required key", structKeyExists(rprops.plainprop, "required"));

// --- default="…" must be PRESERVED in property metadata (Lucee parity) ---
// Previously the parsed `default` expression was never emitted into the
// __properties metadata, so getMetadata().properties[x].default was missing —
// breaking Preside insertData's auto-population of unprovided defaulted fields.
assertTrue("litdefault has default key", structKeyExists(rprops.litdefault, "default"));
assert("litdefault literal preserved", rprops.litdefault.default, "hello default");
assert("cfmldefault cfml: prefix preserved verbatim", rprops.cfmldefault.default, "cfml:Now()");
assert("methoddefault method: prefix preserved verbatim", rprops.methoddefault.default, "method:CalcIt");
assertFalse("plainprop has no default key", structKeyExists(rprops.plainprop, "default"));

suiteEnd();
</cfscript>
