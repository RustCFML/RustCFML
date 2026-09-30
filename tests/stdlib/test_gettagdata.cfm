<cfscript>
suiteBegin("getTagData");

// --- dbinfo metadata is returned (not null) ---
d = getTagData("CF", "DBINFO");
assertFalse("getTagData CF/DBINFO is not null", isNull(d));

// --- library is case-insensitive ---
assertFalse("library is case-insensitive", isNull(getTagData("cf", "dbinfo")));

// --- has an attributes struct ---
assertTrue("dbinfo has attributes struct", isStruct(d.attributes));

// --- Preside feature-detect: filter attribute is supported ---
assertTrue("dbinfo supports filter attribute", structKeyExists(d.attributes, "filter"));

// --- the real supported attributes are present ---
assertTrue("dbinfo supports type", structKeyExists(d.attributes, "type"));
assertTrue("dbinfo supports name", structKeyExists(d.attributes, "name"));
assertTrue("dbinfo supports table", structKeyExists(d.attributes, "table"));
assertTrue("dbinfo supports datasource", structKeyExists(d.attributes, "datasource"));

// --- attribute entries describe the attribute ---
assert("type attribute name", d.attributes.type.nameWithCase, "type");
assertTrue("type attribute is required", d.attributes.type.required);
assertFalse("filter attribute is optional", d.attributes.filter.required);

// --- tag name echoed ---
assert("tag name", d.name, "dbinfo");

// --- unknown tag / non-CF library ---
// Lucee throws "tag [...] is not a built in tag". RustCFML describes only the
// tags it has metadata for (dbinfo), so for anything else it returns null
// rather than wrongly rejecting a real tag.
if ( isRustCFML() ) {
	assertTrue("unknown tag returns null", isNull(getTagData("CF", "NoSuchTag")));
	assertTrue("non-CF library returns null", isNull(getTagData("custom", "anything")));
} else {
	assertThrows("unknown tag throws", function() { getTagData("CF", "NoSuchTag"); });
	assertThrows("non-CF library throws", function() { getTagData("custom", "anything"); });
}

suiteEnd();
</cfscript>
