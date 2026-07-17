// The concrete leaf lives one directory ABOVE the abstract, so the inherited
// implements="sub.IIfaceThing" must NOT resolve against this file's directory
// (that would look for a spurious tests/oop/sub/IIfaceThing.cfc). Mirrors
// ColdBox's LuceeProvider (system/cache/providers/) extending the abstract.
component extends="ifaceinherit.AbstractIfaceProvider" {
}
