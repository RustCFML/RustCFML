component {
    // Unqualified `new SiblingMap()` is LEXICALLY written here. This file is
    // loaded through the /dotdotprobe mapping (mapping name != physical dir
    // "oop"), so its logical package is dotdotprobe.mapfqn.sys. The new sibling
    // must inherit THAT mapping-qualified package — dotdotprobe.mapfqn.sys —
    // NOT the webroot-relative oop.mapfqn.sys the filesystem layout would give.
    // This is the TestBox `testbox.system.Expectation` case.
    function makeSibling(){ return new SiblingMap(); }
    function siblingName(){ return getMetadata( makeSibling() ).name; }
}
