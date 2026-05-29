component {
    this.name = "application-stop-test";

    function onApplicationStart() {
        application.startedByLifecycle = true;
        application.seed = createUUID();
    }

    function onRequest(targetPage) {
        include "#targetPage#";
    }
}
