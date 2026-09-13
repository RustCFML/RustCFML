/**
 * The same bean WITH accessors="true": the declared properties get real
 * generated accessors, and anything undeclared still has none (GH #421).
 */
component accessors="true" {
	property name="username" type="string";
	property name="email"    type="string";
}
