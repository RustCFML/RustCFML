/**
 * A plain data CFC: properties declared, accessors NOT enabled, and no
 * onMissingMethod. CFML synthesizes NO accessors for this shape (GH #421).
 */
component {
	property name="username" type="string";
	property name="email"    type="string";
}
