<cfscript>
// getMetaData(instance) and getComponentMetaData("dotted.path") return the SAME
// cached struct for a class on Lucee: a key a caller adds through one form is
// visible through the other (verified on Lucee 7.1.0.204). RustCFML built and
// cached each form separately, so the two diverged after any mutation, and
// every class's metadata was held twice. The two forms are also identical in
// content: the RustCFML-only `fullExtends` key on the path form is gone.
suiteBegin("getMetaData(instance) and getComponentMetaData(path) share one struct");

mfsKey = "mfs_" & replace(createUUID(), "-", "", "all");

mfsByInstance = getMetaData(new oop.GcmCacheL1());
mfsByPath = getComponentMetaData("oop.GcmCacheL1");

mfsByInstance[mfsKey] = "from-instance";
assertTrue("instance-form key visible through the path form",
    structKeyExists(getComponentMetaData("oop.GcmCacheL1"), mfsKey));

mfsByPath[mfsKey & "_p"] = "from-path";
assertTrue("path-form key visible through the instance form",
    structKeyExists(getMetaData(new oop.GcmCacheL1()), mfsKey & "_p"));

assertFalse("no RustCFML-only fullExtends key", structKeyExists(mfsByPath, "fullExtends"));
assert("same name both ways", mfsByInstance.name, mfsByPath.name);
assert("same function count both ways", arrayLen(mfsByInstance.functions), arrayLen(mfsByPath.functions));

// Leave the shared struct as we found it for the tests that follow.
structDelete(mfsByPath, mfsKey);
structDelete(mfsByPath, mfsKey & "_p");

suiteEnd();
</cfscript>
