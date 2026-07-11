<cfscript>
// java.util.Optional shim. ColdBox's cbproxies Optional.cfc wraps a real
// java.util.Optional (createObject("java","java.util.Optional") in its
// pseudo-constructor, then .empty()/.of()/isPresent()/get()/map()/filter()/
// ifPresent()). RustCFML has no JVM, so this is a value-container shim backed by
// a struct holding a presence flag + single value. This test exercises the raw
// java surface the CFC delegates to. See crates/cfml-vm/src/java_shims.rs
// (make_java_optional) + Vm::handle_java_optional_method.
suiteBegin( "Java Shims: java.util.Optional" );

opt = createObject( "java", "java.util.Optional" );

// empty(): not present, get() throws, isEmpty() true.
e = opt.empty();
assertTrue( "empty().isPresent() false", e.isPresent() == false );
assertTrue( "empty().isEmpty() true", e.isEmpty() == true );
assertThrows( "empty().get() throws", function() { e.get(); } );
assert( "empty().toString()", e.toString(), "Optional.empty" );

// of(v): present, get() returns the value.
p = opt.of( "hello" );
assertTrue( "of().isPresent() true", p.isPresent() == true );
assertTrue( "of().isEmpty() false", p.isEmpty() == false );
assert( "of().get()", p.get(), "hello" );
assert( "of().toString()", p.toString(), "Optional[hello]" );

// map() on a present optional applies the mapper and rewraps.
mapper = createDynamicProxy(
    { apply : function( v ){ return uCase( arguments.v ); } },
    [ "java.util.function.Function" ]
);
mapped = opt.of( "hello" ).map( mapper );
assertTrue( "map() stays present", mapped.isPresent() );
assert( "map() applied", mapped.get(), "HELLO" );

// map() on empty is a no-op (stays empty, mapper not invoked).
mappedEmpty = opt.empty().map( mapper );
assertTrue( "map() on empty stays empty", mappedEmpty.isEmpty() );

// filter() keeps the value when the predicate passes...
keepPred = createDynamicProxy(
    { test : function( v ){ return len( arguments.v ) > 2; } },
    [ "java.util.function.Predicate" ]
);
kept = opt.of( "abcd" ).filter( keepPred );
assertTrue( "filter() keeps matching", kept.isPresent() );
assert( "filter() kept value", kept.get(), "abcd" );

// ...and drops it (→ empty) when the predicate fails.
dropped = opt.of( "ab" ).filter( keepPred );
assertTrue( "filter() drops non-matching", dropped.isEmpty() );

// ifPresent() runs the consumer only when a value is present.
request._optSeen = "";
consumer = createDynamicProxy(
    { accept : function( v ){ request._optSeen = arguments.v; } },
    [ "java.util.function.Consumer" ]
);
opt.of( "fired" ).ifPresent( consumer );
assert( "ifPresent() runs consumer when present", request._optSeen, "fired" );
request._optSeen = "untouched";
opt.empty().ifPresent( consumer );
assert( "ifPresent() skips consumer when empty", request._optSeen, "untouched" );

// equals(): both empty, or both present with equal values.
assertTrue( "equals both empty", opt.empty().equals( opt.empty() ) );
assertTrue( "equals both present eq", opt.of( "x" ).equals( opt.of( "x" ) ) );
assertTrue( "not equals differing value", opt.of( "x" ).equals( opt.of( "y" ) ) == false );
assertTrue( "not equals present vs empty", opt.of( "x" ).equals( opt.empty() ) == false );

// hashCode(): 0 for empty, stable + value-derived for present.
assertTrue( "hashCode empty is 0", opt.empty().hashCode() == 0 );
assertTrue( "hashCode present stable",
    opt.of( "k" ).hashCode() == opt.of( "k" ).hashCode() );

suiteEnd();
</cfscript>
