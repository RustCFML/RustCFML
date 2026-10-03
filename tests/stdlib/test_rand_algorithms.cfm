<cfscript>
suiteBegin("Stdlib: rand/randRange/randomize algorithms + java.util.UUID (GH ##459)");

// Measured against Lucee 7.1.0.204.
assertThrows("randRange rejects an unknown algorithm", function() { randRange(0, 15, "BOGUSALGO"); });
assertThrows("rand rejects an unknown algorithm", function() { rand("BOGUSALGO"); });
assertThrows("randomize rejects an unknown algorithm", function() { randomize(1, "BOGUSALGO"); });

r = randRange(0, 15, "cfmx_compat");
assertTrue("cfmx_compat accepted (case-insensitive)", r >= 0 && r <= 15);
r = randRange(0, 15, "sha1prng");
assertTrue("SHA1PRNG accepted (case-insensitive)", r >= 0 && r <= 15);
r = randRange(0, 15, "NativePRNG");
assertTrue("NativePRNG accepted", r >= 0 && r <= 15);

// The default algorithm is seedable...
randomize(42); a = randRange(0, 1000000); randomize(42); b = randRange(0, 1000000);
assert("randomize(seed) makes the default algorithm repeat", a, b);
// ...and randomize() returns the first number of the seeded sequence, not 0.
x = randomize(42); y = randomize(42);
assert("randomize() returns the same number for the same seed", x, y);
assertTrue("randomize() returns a number in [0,1)", x >= 0 && x < 1);

// ...but SHA1PRNG is secure: reseeding does not make it repeat.
same = 0;
for (i = 1; i <= 5; i++) {
	randomize(42); a = randRange(0, 1000000000, "SHA1PRNG");
	randomize(42); b = randRange(0, 1000000000, "SHA1PRNG");
	if (a == b) same++;
}
assertTrue("SHA1PRNG is not affected by randomize()", same < 5);

randomize(42); a = createUUID(); randomize(42); b = createUUID();
assertFalse("createUUID is not affected by randomize()", a == b);
randomize(42); a = createGUID(); randomize(42); b = createGUID();
assertFalse("createGUID is not affected by randomize()", a == b);

u = createObject("java", "java.util.UUID").randomUUID().toString();
assertTrue("java UUID.randomUUID() is a v4 UUID", reFind("^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$", u) == 1);
assertFalse("java UUID.randomUUID() has random high bits", left(u, 8) == "00000000" && mid(u, 10, 4) == "0000");
u2 = createObject("java", "java.util.UUID").randomUUID().toString();
assertFalse("two random UUIDs differ", u == u2);

suiteEnd();
</cfscript>
