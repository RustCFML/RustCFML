component {
	// A nested `new` inside a STRUCT LITERAL in the pseudo-constructor: the
	// shape whose stack residue used to poison the enclosing component.
	variables._services = { t = new sub.Thrower() };
}
