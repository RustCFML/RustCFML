<cfscript>
// GH #304 — getLocale()/setLocale() were inert stubs and cfconfig runtime.locale
// had no consumer, so every ls* function was pinned to en_US.
//
// Locale is request state now, exactly like the timezone. Expectations verified
// against Lucee 7.0.4.34 via CommandBox.
suiteBegin("Locale state (GH ##304)");

original = getLocale();

// Establish a known baseline first: the configured default differs by engine
// (RustCFML reads tests/.cfconfig.json, Lucee does not), so asserting against it
// would make this suite pass on one engine only.
setLocale("en_US");

// setLocale() returns the PREVIOUS locale, in CODE form — that asymmetry with
// getLocale() (friendly name) is what makes save-and-restore work, and is Lucee's
// documented contract.
previous = setLocale("en_GB");
assert("setLocale returns the PREVIOUS locale", previous, "en_US");
assert("setLocale actually takes effect", getLocale(), "english (uk)");

// The whole point: the ls* family must follow the locale.
assert("lsCurrencyFormat follows the locale", lsCurrencyFormat(1234.5), "£1,234.50");
assert("lsCurrencyFormat international form", lsCurrencyFormat(1234.5, "international"), "GBP 1,234.50");
assert("lsCurrencyFormat wraps negatives in parens", lsCurrencyFormat(-99.99), "(£99.99)");

restored = setLocale(original);
assert("setLocale round-trips", restored, "en_GB");
assert("locale is back to where it started", getLocale(), original);

// Every spelling CFML accepts must resolve to the same locale.
for (spelling in ["en_GB", "en-GB", "English (UK)"]) {
    setLocale(spelling);
    assert("setLocale accepts [#spelling#]", getLocale(), "english (uk)");
}
setLocale(original);

// An explicit locale ARGUMENT must be honoured too — it used to be parsed and
// dropped, so lsCurrencyFormat(1234.5, "local", "de_DE") returned "$1,234.50".
assert("explicit locale arg: de_DE", lsCurrencyFormat(1234.5, "local", "de_DE"), "1.234,50 €");
assert("explicit locale arg: en_GB", lsCurrencyFormat(1234.5, "local", "en_GB"), "£1,234.50");
// JPY has no minor unit, and rounds HALF-UP like Java (not half-to-even).
// Note the FULLWIDTH yen sign (U+FFE5) — Java/Lucee use it for ja_JP, not ¥.
assert("explicit locale arg: ja_JP has no minor unit", lsCurrencyFormat(1234.5, "local", "ja_JP"), "￥1,235");
assert("lsNumberFormat re-punctuates for the locale", lsNumberFormat(1234567.891, "9,999.99", "de_DE"), "1.234.567,89");

// An unresolvable locale must be an ERROR. Silently formatting as en_US is the
// exact failure mode this issue is about — the caller would never learn.
assertThrows("unknown locale argument throws", function() {
    return lsCurrencyFormat(1, "local", "not a real locale");
});
assertThrows("setLocale with an unknown locale throws", function() {
    return setLocale("not a real locale");
});

assert("locale unchanged after the failed calls", getLocale(), original);

suiteEnd();
</cfscript>
