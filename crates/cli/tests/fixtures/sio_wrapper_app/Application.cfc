component {

	this.name = "sio_wrapper_app";

	/**
	 * Bootstrap the socket.io-lucee-style server once, on first request — the
	 * app owns its bootstrap, as that library requires.
	 */
	public boolean function onApplicationStart() {
		application.io = new SioFixtureServer();

		return true;
	}
}
