/**
 * Fixture for relative file-BIF resolution (Lucee parity).
 *
 * Verified on Lucee 7: a relative file path passed to a file BIF from inside a
 * component resolves against the request's BASE TEMPLATE directory (the page
 * that started the request), NOT this component's own directory — the same base
 * ExpandPath uses. This CFC lives in a subdirectory, so a relative read here
 * must NOT find a sibling file; it must find the file next to the base template.
 * (Supersedes the earlier GitHub #171 behaviour, which resolved against the
 * CFC's own directory and diverged from Lucee.)
 */
component {

	public function init(){
		return this;
	}

	// dot-relative
	public string function readDotRelative(){
		return fileRead( "./rel_bif_probe.json" );
	}

	// bare relative (no ./)
	public string function readBareRelative(){
		return fileRead( "rel_bif_probe.json" );
	}

	// fileExists must use the same base
	public boolean function existsRelative(){
		return fileExists( "./rel_bif_probe.json" );
	}

	// the ExpandPath-wrapped read must agree with the bare relative read
	public string function readViaExpandPath(){
		return fileRead( expandPath( "./rel_bif_probe.json" ) );
	}
}
