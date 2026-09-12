component {
  function init() { variables.moduleRoutes = {}; return this; }
  function getModuleRoutes(required string module) {
    if (!structKeyExists(variables.moduleRoutes, arguments.module)) variables.moduleRoutes[arguments.module] = [];
    return variables.moduleRoutes[arguments.module];
  }
}
