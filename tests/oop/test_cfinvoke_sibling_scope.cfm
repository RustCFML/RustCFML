<cfscript>
suiteBegin("cfinvoke sibling in-scope write propagation");

// A no-component cfinvoke (Lucee in-scope form, via attributeCollection) must
// run the sibling method against the LIVE this/variables of the calling
// component, so the callback's mutations survive. Regression for the Wheels
// model-callback bucket (this.setByCallback never appeared on the object).
obj = new oop.CfinvokeSiblingFixture();
obj.fireViaAttrCollection();
assertTrue("this.X set by no-component cfinvoke sibling propagates", structKeyExists(obj, "calledFlag") && obj.calledFlag);

// onMissingMethod fallback for a no-component cfinvoke to an undefined method.
// RustCFML-only: Lucee 7.0/7.1 throw "variable [noSuchMethod] doesn't exist"
// here instead of routing to onMissingMethod.
if ( isRustCFML() ) {
	obj2 = new oop.CfinvokeSiblingFixture();
	obj2.fireMissingViaAttrCollection();
	assertTrue("no-component cfinvoke routes missing method to onMissingMethod on live instance", structKeyExists(obj2, "ommName") && obj2.ommName == "noSuchMethod");
} else {
	assertThrows("no-component cfinvoke of a missing method throws", function() { new oop.CfinvokeSiblingFixture().fireMissingViaAttrCollection(); });
}

suiteEnd();
</cfscript>
