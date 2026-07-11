component {
    public string function register( required IShape shape ) { return "ok:" & shape.area(); }
    public string function registerFqn( required oop.argtype.IShape shape ) { return "ok:" & shape.area(); }
}
