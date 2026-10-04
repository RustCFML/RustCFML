component extends="DeclProtoBase" implements="DeclProtoIface" accessors=true {
    property name="label" type="string" default="L";
    property name="count" type="numeric" default="0";
    static { created = 0; }
    public any function init(string name = "n") {
        variables.name = arguments.name;
        static.created++;
        variables.inits = (variables.inits ?: 0) + 1;
        variables.opts = {};
        variables.tags = [];
        return this;
    }
    function ping() { return "child>" & super.ping(); }
    function getName() { return variables.name; }
    function getOpts() { return variables.opts; }
    function getTags() { return variables.tags; }
    function getInits() { return variables.inits; }
    function selfViaVariablesThis() { return variables.this.getName(); }
    function staticCount() { return static.created; }
}
