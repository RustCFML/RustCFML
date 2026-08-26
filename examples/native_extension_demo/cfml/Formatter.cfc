/**
 * A CFC shipped INSIDE the extension.
 *
 * The engine mounts the extension's `cfml/` directory as `/demo/`, so this is
 * reachable as `demo.Formatter` from any application with the extension
 * installed — no file in the application, no mapping to configure.
 *
 * This is what makes a .rcx an *extension* rather than a plugin: a Rust core
 * can present a CFML facade, which is usually the nicer API anyway.
 */
component {

    public string function slug( required string text ) {
        // Straight through to the extension's Rust implementation.
        return slugify( arguments.text );
    }

    public array function slugAll( required array items ) {
        return arrayMap( arguments.items, function( item ) { return slugify( item ); } );
    }
}
