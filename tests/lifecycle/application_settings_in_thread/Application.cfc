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

    // The engine's own reader of an application setting: the s3*() builtins
    // take their defaults from this.s3. A NON-RESOLVING host and the AWS
    // documentation example keypair, so presigning proves the setting arrived
    // without a bucket or a network (see tests/s3/test_s3_app_config.cfm).
    this.s3 = {
          accessKeyId  = "AKIAIOSFODNN7EXAMPLE"
        , awsSecretKey = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        , host         = "settings-in-thread-probe.example.com"
    };
}
