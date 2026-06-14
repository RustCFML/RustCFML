// Empty subclass in a DIFFERENT directory (oop/inhsub/) than its parent
// (oop/inh/). Inherits viaCreate(); calling it must still resolve the bare
// "InhSibling" against the PARENT's dir, not this subclass's dir.
component extends="oop.inh.InhParent" {}
