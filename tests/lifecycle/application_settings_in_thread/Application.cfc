/**
 * Fixture: an application that declares keys of its own on `this`, beside the
 * standard ones. getApplicationSettings() must report them wherever it is
 * called from, a cfthread included.
 */
component {
    this.name = "rcfml_settings_in_thread_fixture";
    this.sessionManagement = false;

    // Keys of the application's own: a simple value and a struct.
    this.projectLabel = "kept";
    this.storage = { host: "https://storage.example.com", region: "auto" };
}
