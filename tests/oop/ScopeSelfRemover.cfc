/**
 * Regression fixture: a component stored in a scope whose method removes its
 * own entry from that scope. The engine's post-call receiver ("this"-state)
 * write-back must NOT resurrect the just-removed entry — matching CFML
 * reference semantics. This is the minimal shape of the Preside "reload all"
 * bug, where `application.cbBootstrap.onRequestStart()` internally ran
 * `application.clear()` and the dispatch write-back resurrected cbBootstrap
 * into an otherwise-cleared application scope (leaving cbController undefined).
 */
component {
    function removeMeFromApplication( required string key ) {
        structDelete( application, arguments.key );
        return "removed";
    }
}
