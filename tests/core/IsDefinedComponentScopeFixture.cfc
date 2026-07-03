// Fixture: isDefined() must resolve an UNSCOPED variable assigned inside a CFC
// method (or a closure defined in one). Such a var lands in the component
// `__variables` scope, not the function-local frame — the variable READ path
// checks __variables, so isDefined() must too. Previously isDefined("posts")
// returned false for it (only isDefined("variables.posts") worked), which broke
// Wheels crudSpec's calc-property test: isDefined("posts.titleAlias") on an
// unscoped query variable.
component {
	public struct function probe() {
		posts = queryNew("id,titleAlias", "integer,varchar");   // unscoped -> __variables
		plainVar = "hello";
		return {
			// unqualified name of an unscoped var
			bareVar        = isDefined("plainVar"),
			bareQuery      = isDefined("posts"),
			queryColumn    = isDefined("posts.titleAlias"),
			queryMissingCol= isDefined("posts.nope"),
			// explicit variables. prefix must still work
			scopedVar      = isDefined("variables.posts"),
			scopedColumn   = isDefined("variables.posts.titleAlias"),
			// truly-undefined must stay false
			undefined      = isDefined("noSuchVar987")
		};
	}

	// same, but the var is assigned inside a CLOSURE defined in the method
	public struct function probeClosure() {
		var cb = () => {
			qq = queryNew("id,titleAlias", "integer,varchar");
			return { bare = isDefined("qq"), col = isDefined("qq.titleAlias") };
		};
		return cb();
	}
}
