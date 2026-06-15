component {
    public string function doSomething() {
        return "did-something";
    }
    public any function onMissingMethod(required string missingMethodName, struct missingMethodArguments = {}) {
        return "omm:" & arguments.missingMethodName;
    }
}
