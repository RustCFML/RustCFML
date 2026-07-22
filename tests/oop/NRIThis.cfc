component accessors="true" {
    property name="tag" default="";
    function init(){ this.pub="PUB"; variables.priv="PRIV"; setTag("TAG"); return this; }
    function getPub(){ return this.pub; } function getPriv(){ return variables.priv; }
}
