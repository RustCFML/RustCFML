<cffunction name="gh259SharedHelper"><cfreturn structKeyExists(variables,"controller") ? variables.controller : "MISSING"></cffunction>
