component {
    // An application is what makes the `application` scope exist and persist
    // across requests — which is the whole point of the tier-2 examples here.
    this.name = "rustcfml_extension_demo";
    this.applicationTimeout = createTimeSpan( 0, 1, 0, 0 );
}
