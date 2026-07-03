<cfscript>
// Language/metadata fixes uncovered while booting Preside on RustCFML. Every
// case below was cross-checked against Lucee 7 (the reference engine); the
// expected values are Lucee's. Grouped by the engine area they exercise.
suiteBegin("Preside boot language & metadata fixes");

// --- 1. Reading a declared-but-unpassed optional `arguments` member is NULL,
//        not an "undefined variable" throw (Lucee/ACF). Undeclared keys throw.
//        (Preside FeatureService.isFeatureEnabled recurses with an optional
//        `arguments.siteTemplate` it never passes.)
optArgReads = function( required string a, string b ) {
    // In a string context the unpassed arg coerces to "" without throwing.
    return "[" & arguments.b & "]" & "|ske=" & structKeyExists( arguments, "b" );
};
assert( "declared-but-unpassed arg reads empty, no throw", optArgReads( "x" ), "[]|ske=false" );

optArgThrowsUndeclared = function() {
    // Truly-undeclared arguments key still throws.
    try {
        return "no-throw:" & arguments.totallyUndeclared;
    } catch ( any e ) {
        return "threw";
    }
};
assert( "undeclared arguments key still throws", optArgThrowsUndeclared(), "threw" );

// --- 2. getComponentMetaData / getMetaData always carry `type="component"`, and
//        `.extends` is a recursive STRUCT (name/type), not a bare parent string.
//        (ColdBox Util.getInheritedMetaData walks md.type / md.extends.name.)
bootMdChild = getComponentMetaData( "core.boot_md_child" );
assert( "getComponentMetaData has type", bootMdChild.type ?: "MISSING", "component" );
assertTrue( "extends is a struct", isStruct( bootMdChild.extends ?: "" ) );
// name may be bare (RustCFML) or package-qualified (Lucee `core.boot_md_parent`).
assertTrue( "extends.name identifies the parent", findNoCase( "boot_md_parent", bootMdChild.extends.name ?: "" ) > 0 );
assert( "extends.type present", bootMdChild.extends.type ?: "MISSING", "component" );

// --- 3. Any reserved keyword is a legal identifier after `var` (Lucee/ACF).
kwIdents = function() {
    var catch    = "c";
    var type     = "t";
    var switch   = "s";
    var abort    = "a";
    var finally  = "f";
    return catch & type & switch & abort & finally;
};
assert( "keywords usable as var identifiers", kwIdents(), "ctsaf" );

// A keyword-named variable is also a valid EXPRESSION base, including member
// access — Preside's errorTemplate.cfm reads `catch.message` / `catch.type`.
kwMemberRead = function() {
    var catch = { message="boom", type="Custom" };
    return catch.message & "/" & catch.type;
};
assert( "keyword-named var member access reads", kwMemberRead(), "boom/Custom" );

// --- 4. cfloop( attrs ){ body } script-block form, including scope-prefixed
//        item/index loop variables (`item="local.v"`, `index="local.i"`).
cfloopBlock = function( data ) {
    var total   = 0;
    var lastIdx = 0;
    cfloop( array=arguments.data, item="local.v", index="local.i" ) {
        total += v;
        lastIdx = i;
    }
    return total & "/" & lastIdx;
};
assert( "cfloop(array,item,index) block w/ scoped vars", cfloopBlock( [ 10, 20, 30 ] ), "60/3" );

// --- 5. `elseif` (one word) in cfscript, including a standalone elseif that is
//        FOLLOWED by a terminal `else`.
ifChain = function( x ) {
    if ( x == 1 ) { return "one"; }
    elseif ( x == 2 ) { return "two"; }
    elseif ( x == 3 ) { return "three"; }
    else { return "other"; }
};
assert( "elseif chain -> two",   ifChain( 2 ), "two" );
assert( "elseif chain -> three", ifChain( 3 ), "three" );
assert( "elseif chain -> else",  ifChain( 9 ), "other" );

// --- 6. Untyped property metadata defaults `type` to "any" (Lucee parity);
//        Preside PresideObjectReader reads `prop.type` directly.
propMd = getMetaData( createObject( "component", "core.boot_prop_component" ) );
assert( "untyped property type defaults to any", propMd.properties[ 1 ].type ?: "MISSING", "any" );
assert( "typed property keeps its type",         propMd.properties[ 2 ].type ?: "MISSING", "string" );

suiteEnd();
</cfscript>
