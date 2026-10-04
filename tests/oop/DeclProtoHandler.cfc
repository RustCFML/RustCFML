component {
    this.allowedMethods = { save: "POST", remove: "POST,DELETE" };
    this.prehandler_only = "index,edit";
    variables.limit = 10;
    variables.cols = [ "a", "b" ];
    mode = "bare";
    public any function init() { return this; }
    function getLimit() { return variables.limit; }
    function getCols() { return variables.cols; }
    function getMode() { return mode; }
    function setLimit(n) { variables.limit = n; }
}
