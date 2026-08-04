/**
 * Fixture for docs/known-issues.md §1 — `this.timezone` and `this.locale` were
 * captured into the app's settings struct and then read by nothing, so an app
 * declaring them formatted every date and number in the SERVER's zone/locale
 * while getApplicationSettings() claimed otherwise.
 *
 * de_DE is chosen because both engines agree on its friendly name AND its number
 * punctuation; Lucee's reverse name lookup misses some codes (fr_FR echoes back
 * as "fr_fr"), which is a §26 locale-table matter, not this one.
 */
component {
    this.name     = "rcfml_tz_locale_fixture";
    this.timezone = "Asia/Tokyo";
    this.locale   = "de_DE";
}
