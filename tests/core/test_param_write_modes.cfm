<!--- §109/§110: bare writes to a parameter — `&=` writes through to `arguments` under
      localmode="modern" (the only compound op that does); a bare write to a DELETED
      parameter lands in `variables` under classic localmode. Lucee 7.1 parity. --->
<cfscript>
suiteBegin("Parameter writes: &= write-through (modern) and deleted-param routing (classic)");

pwm = new ParamWriteModes();

assert("modern: a &= X updates arguments.a and creates no local.a", pwm.concatModern("A"), "no-local args.a=AX bare=AX");
assert("modern: a += 1 is a local write, argument untouched", pwm.plusModern(1), "local.a=2 args.a=1 bare=2");
assert("modern: a = a & X is a local write, argument untouched", pwm.assignModern("A"), "local.a=AX args.a=A bare=AX");
assert("modern: two &= both write through", pwm.concatTwiceModern("A"), "args.a=AXY bare=AXY");
assert("modern: &= then = : the = is local, argument keeps the &= result", pwm.concatThenAssignModern("A"), "args.a=AX bare=Z");
assert("modern: = then &= : the name is already local, &= stays local", pwm.assignThenConcatModern("A"), "args.a=A bare=ZX local.a=ZX");
assert("modern: var a then &= is local", pwm.varThenConcatModern("A"), "args.a=A bare=VX");
assert("modern: &= on a non-parameter is a local write", pwm.nonParamConcatModern(), "local.b=BX vars.b=none");
assert("classic: a &= X writes through (as every bare write does)", pwm.concatClassic("A"), "args.a=AX bare=AX");
pwm.clearVars();
assert("classic: bare write after structDelete(arguments) lands in variables", pwm.deleteThenWriteClassic("A"), "vars.a=W local.a=none args.a=none");
pwm.clearVars();
assert("modern: bare write after structDelete(arguments) is local", pwm.deleteThenWriteModern("A"), "vars.a=none local.a=W args.a=none");
pwm.clearVars();
assert("classic: &= after the delete follows the variable, not the argument", pwm.deleteThenConcatClassic("A"), "vars.a=WV args=false");
pwm.clearVars();

suiteEnd();
</cfscript>
