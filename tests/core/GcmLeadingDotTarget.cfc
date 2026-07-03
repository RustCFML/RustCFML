/**
 * Fixture for test_gcm_leading_dot_path.cfm — exercises getComponentMetaData()
 * on a leading-dot dotted path (".core.GcmLeadingDotTarget"), the form Preside's
 * TaskManagerConfigurationWrapper builds via Replace( filePath, "/", ".", "all" ).
 */
component {
	/**
	 * @schedule 0 0 * * *
	 */
	public void function scheduledThing() {}

	public string function plainThing() {
		return "ok";
	}
}
