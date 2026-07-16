<cfscript>
// Regression: a <cfproperty> WITHOUT accessors="true" must NOT synthesize
// implicit getX/setX accessors. When the component defines onMissingMethod,
// getX/setX must route THERE (Lucee/ACF parity). RustCFML previously treated a
// declared-but-unset property as a "known member" and ran a lenient implicit
// setter that wrote the wrong scope — which silently emptied Masa CMS's ORM
// table name (mura.bean.bean routes setTable→onMissingMethod→instance.table),
// producing cfdbinfo "Missing attribute [table]. The type [columns] requires
// the attribute [table]." on the Approval Chains / Web Services / Staging admin
// pages. Cross-checked on Lucee 7.
suiteBegin("Property without accessors routes to onMissingMethod");

o = createObject("component", "PropNoAccessorsOmm");

// setTable() has NO real method and accessors is off → must reach onMissingMethod,
// which stores into variables.instance.table.
o.setTable("tapprovalchains");
assert("setX routes to onMissingMethod (stores instance.table)",
	o.readInstanceTable(), "tapprovalchains");
assertTrue("onMissingMethod was actually invoked for setTable", o.ommHits() >= 1);

// getTable() must also route to onMissingMethod and read back the same value.
assert("getX routes to onMissingMethod and returns instance value",
	o.getTable(), "tapprovalchains");

// A second property behaves identically.
o.setKeyField("id");
assert("second dynamic setter also routes to onMissingMethod",
	o.getKeyField(), "id");

// Guard against over-correction: accessors="true" must STILL synthesize real
// accessors that win over onMissingMethod (which would throw if reached).
withAcc = createObject("component", "PropWithAccessorsFixture");
withAcc.setTable("realtable");
assert("accessors=true uses generated setter/getter, not onMissingMethod",
	withAcc.getTable(), "realtable");

suiteEnd();
</cfscript>
