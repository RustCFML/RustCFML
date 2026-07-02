<!---
  Regression: a closure defined in a CFC method whose for-loop uses an UNSCOPED
  counter. In a CFC method (classic localmode) an unscoped write lands in the
  component scope (__variables); `i++` / `i += k` / `i--` must still mutate it.
  The fused Increment/Decrement/AddLocalConst/MulLocalConst ops previously only
  looked at `locals`, so they silently no-opped when the var lived in __variables
  — making `for (i=1; i lte n; i++)` loop forever. This was the Wheels
  view.assetsSpec "returns same domain for asset" hang that stalled the whole
  TestBox suite. Passes identically on Lucee 7.
--->
<cfscript>
suiteBegin("Closure unscoped loop-var increment in CFC method");

f = new ClosureLoopVarFixture();

// `i++` on an unscoped (component-scope) counter inside a CFC-method closure
// must terminate at i=6 after 5 iterations — not loop forever.
assert("unscoped i++ in CFC-method closure terminates", f.runLoop(), "i=6,cnt=5");

// `i += 2` (AddLocalConst) and `down--` (Decrement) on unscoped component-scope
// vars must also mutate, not no-op.
assert("unscoped += and -- in CFC-method closure", f.runDelta(), "i=10,iters=5,down=1");

// FUSED counted-loop shape (constant limit + i++ → ForLoopStep): an unscoped
// counter in __variables must not run the first iteration twice. `lte 4` yields
// exactly 4 elements, not 5.
assert("fused counted loop, unscoped counter in CFC-method closure", f.runCounted(), "1,2,3,4");

// Same, reached through a stored/re-invoked closure. 30 iterations → length 30.
assert("fused counted loop via nested closure", f.runCountedNested(), 30);

suiteEnd();
</cfscript>
