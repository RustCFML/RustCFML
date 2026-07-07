<!---
  GitHub #255: closure / arrow-function parameter DEFAULTS were never applied
  when the defaulted param was omitted at the call site (named `function`
  declarations already worked). Regressed the ColdBox ioc suite via LogBox
  FileAppender's `variables.lock = function( type="exclusive", body ){...}`
  closure, called as `variables.lock( body=... )` with `type` defaulted.

  Root cause: the closure/arrow codegen emitted the OLD default-preamble shape
  (`LoadLocal(param); IsNull; JumpIfFalse`). Since v0.408 the VM stopped
  pre-seeding an omitted param as a Null local and undefined reads THROW, so
  `LoadLocal` on an absent param raised `Variable 'X' is undefined` before the
  default could ever be applied. Named functions were already switched to the
  `JumpIfArgPresent` (arguments-scope presence) shape for #240; this brings
  closures and arrow functions in line. Verified against Lucee 7.
--->
<cfscript>
suiteBegin("Closure/arrow parameter defaults (GitHub 255)");

// --- closures: default applied when param omitted ---
greetClosure = function( name="world" ){ return "hello " & name; };
assert("closure default applied on zero-arg call", greetClosure(), "hello world");
assert("closure default overridden when supplied", greetClosure("bob"), "hello bob");
assert("closure default overridden via named arg", greetClosure(name="sue"), "hello sue");

// --- arrow functions: same ---
greetArrow = ( name="world" ) => { return "hello " & name; };
assert("arrow default applied on zero-arg call", greetArrow(), "hello world");
assert("arrow default overridden when supplied", greetArrow("bob"), "hello bob");

// --- the exact FileAppender shape: defaulted param + required param, called
//     with only the required one by name ---
lockShape = function( type="exclusive", body ){ return type & "/" & body; };
assert("defaulted-first param defaults when only later param passed by name",
       lockShape( body="B" ), "exclusive/B");
assert("defaulted-first param honored when passed explicitly",
       lockShape( type="readonly", body="B" ), "readonly/B");

// --- default is last param ---
lastDefault = function( a, b="def" ){ return a & "/" & b; };
assert("trailing default applied", lastDefault("A"), "A/def");
assert("trailing default overridden", lastDefault("A", "B"), "A/B");

// --- two defaults, call with none / some ---
twoDefaults = function( x="one", y="two" ){ return x & "," & y; };
assert("both defaults on zero-arg", twoDefaults(), "one,two");
assert("first supplied, second defaults", twoDefaults("X"), "X,two");
assert("both supplied", twoDefaults("X","Y"), "X,Y");

// --- argumentCollection with defaulted param omitted ---
argColl = function( type="exclusive", body="none" ){ return type & "/" & body; };
assert("argumentCollection omitting defaulted param applies default",
       argColl(argumentCollection={}), "exclusive/none");
assert("argumentCollection supplying param wins",
       argColl(argumentCollection={type:"shared"}), "shared/none");

// --- default expression referencing an enclosing variable (must not be masked
//     by the #240 self-name fix) ---
outer = "captured";
capturingDefault = function( a=outer ){ return a; };
assert("closure default expr reads enclosing var", capturingDefault(), "captured");

// --- self-named default in a closure: f = function(a=a) reads enclosing `a` ---
a = "enclosingA";
selfNamed = function( a=a ){ return a; };
assert("closure self-named default reads enclosing (GH240 parity)", selfNamed(), "enclosingA");

// --- the defaulted param is visible in the arguments scope too ---
argsVisible = function( name="world" ){ return arguments.name; };
assert("defaulted param materialized in arguments scope", argsVisible(), "world");

// --- default expression is a computed value, evaluated on omission ---
computedDefault = function( n=1+2 ){ return n; };
assert("computed default expression evaluated", computedDefault(), 3);

// --- nested closure with its own defaulted param ---
makeAdder = function( base=10 ){
    return function( inc=5 ){ return base + inc; };
};
assert("nested closure defaults both apply", makeAdder()(), 15);
assert("nested closure inner default overridden", makeAdder(100)(1), 101);

suiteEnd();
</cfscript>
