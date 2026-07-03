<cfscript>
suiteBegin("isDefined resolves unscoped vars in the component (__variables) scope");

isdcs = new IsDefinedComponentScopeFixture();
r = isdcs.probe();

assertTrue("isDefined finds unscoped simple var in CFC method", r.bareVar);
assertTrue("isDefined finds unscoped query var in CFC method", r.bareQuery);
assertTrue("isDefined finds column on unscoped query (Wheels calc-property)", r.queryColumn);
assertFalse("isDefined false for missing column on unscoped query", r.queryMissingCol);
assertTrue("isDefined variables.posts still works", r.scopedVar);
assertTrue("isDefined variables.posts.titleAlias still works", r.scopedColumn);
assertFalse("isDefined false for a truly undefined name", r.undefined);

rc = isdcs.probeClosure();
assertTrue("isDefined finds unscoped var assigned in a closure", rc.bare);
assertTrue("isDefined finds query column via closure-scoped var", rc.col);

suiteEnd();
</cfscript>
