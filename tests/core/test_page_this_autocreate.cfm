<cfscript>
// On a page (no component), `this` is not defined until something writes to it;
// the first `this.x = v` creates `variables.this` as a struct (Lucee 7.1).
// RustCFML threw "variable [THIS] doesn't exist" on the write.
suiteBegin("Page-level this.x = v creates variables.this");

ptWasDefined = structKeyExists(variables, "this");
this.ptC = 1;
assertTrue("variables.this exists after the write", structKeyExists(variables, "this"));
assert("value readable as this.x", this.ptC, 1);
assert("value readable as variables.this.x", variables.this.ptC, 1);
assertFalse("the key is not a variables key", structKeyExists(variables, "ptC"));
this.ptD.e = 2;
assert("nested write builds the chain", this.ptD.e, 2);
ptAlias = this;
ptAlias.ptK = 9;
assertTrue("this is a reference-typed struct", structKeyExists(this, "ptK"));
function ptReadThis() { return this.ptC; }
assert("a page function sees it", ptReadThis(), 1);
// Leave the runner as it was if `this` did not exist before.
if (!ptWasDefined) {
    structDelete(variables, "this");
} else {
    structDelete(this, "ptC"); structDelete(this, "ptD"); structDelete(this, "ptK");
}

suiteEnd();
</cfscript>
