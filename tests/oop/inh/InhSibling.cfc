// Sibling co-located with InhParent (tests/oop/inh/). Reached by a bare
// CreateObject("component","InhSibling") from InhParent's method.
component { public string function hi() { return "inh-sibling-ok"; } }
