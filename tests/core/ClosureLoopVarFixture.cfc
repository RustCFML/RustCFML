/*
 * Regression fixture: a closure defined in a CFC method, whose for-loop uses an
 * UNSCOPED counter `i`. In a CFC method under classic localmode an unscoped write
 * lands in the component scope (`__variables`), and `i++` must still increment it.
 * Before the fix, the fused Increment/Decrement/+=/*= ops only looked at `locals`,
 * so `i++` silently no-opped and the loop ran forever (Wheels view.assetsSpec hang).
 */
component {
	function trigger() {
		return true;
	}

	// Mirrors the assetsSpec shape: a sibling closure sets a shared (unscoped)
	// object, the body closure loops with an unscoped counter and calls a method
	// each iteration. `cnt` is var-declared (reliable guard); `i`/`iEnd`/`obj` are
	// unscoped (land in the component scope).
	function runLoop() {
		setup = () => {
			obj = this;
		};
		body = () => {
			var cnt = 0;
			iEnd = 5;
			for (i = 1; i lte iEnd; i++) {
				cnt = cnt + 1;
				if (cnt gt 100) {
					return "RUNAWAY-i-not-incrementing";
				}
				obj.trigger();
			}
			return "i=" & i & ",cnt=" & cnt;
		};
		setup();
		return body();
	}

	// Exercise the FUSED counted-loop shape: a CONSTANT limit + `i++` compiles to
	// ForLoopStep / JumpIfLocalCmpConstFalse (not Increment). Those fused ops read
	// and write only `locals`, so when the unscoped counter lives in the component
	// scope (`__variables`) the first step missed it, re-seeded `locals.i = step`,
	// and the first iteration ran TWICE — a 4-count loop produced 5 elements
	// (Wheels model.miscellaneousSpec `objectid` off-by-one). Passes on Lucee 7.
	function runCounted() {
		body = () => {
			out = [];
			for (i = 1; i lte 4; i++) {
				arrayAppend(out, i);
			}
			return arrayToList(out);
		};
		return body();
	}

	// Same fused shape, but reached through a stored/re-invoked closure (deeper
	// capture, mirroring TestBox's describe/it registration).
	function runCountedNested() {
		specs = [];
		reg = (f) => {
			arrayAppend(specs, f);
		};
		reg(() => {
			out = [];
			for (i = 1; i lte 30; i++) {
				arrayAppend(out, i);
			}
			return arrayLen(out);
		});
		return specs[1]();
	}

	// Same, but exercise `+=` (AddLocalConst) and `--` (Decrement) on unscoped
	// component-scope vars too.
	function runDelta() {
		body = () => {
			var iters = 0;
			n = 0;
			down = 6;
			for (i = 0; i lt 10; i += 2) {
				iters = iters + 1;
				down--;
				if (iters gt 100) {
					return "RUNAWAY-delta";
				}
			}
			return "i=" & i & ",iters=" & iters & ",down=" & down;
		};
		return body();
	}
}
