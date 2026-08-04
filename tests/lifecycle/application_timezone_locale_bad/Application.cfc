/**
 * An unusable timezone id / locale must be IGNORED, falling back to the server
 * default rather than throwing — Lucee 7's verified behaviour, and the safe one
 * (throwing here would fail application startup, not one date call).
 */
component {
    this.name     = "rcfml_tz_locale_bad_fixture";
    this.timezone = "Not/AZone";
    this.locale   = "xx_YY";
}
