<cfscript>
suiteBegin("getTempFile: a new file, never a path handed out twice");

// Its own directory, so the files it makes can be counted and cleaned up.
gtfDir = getTempDirectory() & "rcfml_gettempfile_" & createUUID();
directoryCreate(gtfDir);

try {
	gtfFirst = getTempFile(gtfDir, "gtf");
	assertTrue("the file exists as soon as getTempFile returns", fileExists(gtfFirst));
	assert("the file is empty", fileExists(gtfFirst) ? fileRead(gtfFirst) : "(no file)", "");
	assert("it is in the directory asked for", getDirectoryFromPath(gtfFirst), gtfDir & "/");
	assertTrue("the name starts with the prefix", left(getFileFromPath(gtfFirst), 3) == "gtf");
	assert("the name ends in .tmp", listLast(gtfFirst, "."), "tmp");

	// One fast loop: names were the prefix plus the clock, which repeated
	// whenever two calls fell in the same microsecond (macOS).
	gtfLoop = {};
	for (gtfI = 1; gtfI <= 500; gtfI++) {
		gtfLoop[getTempFile(gtfDir, "gtf")] = true;
	}
	assert("500 calls in a loop give 500 different files", structCount(gtfLoop), 500);

	// Threads at once, as concurrent requests are: Preside's derivative
	// generator got one temp file for two images and stored the same image
	// for both.
	gtfThreads = "";
	for (gtfT = 1; gtfT <= 6; gtfT++) {
		gtfName = "gtf_" & replace(createUUID(), "-", "", "all");
		gtfThreads = listAppend(gtfThreads, gtfName);
		thread name="#gtfName#" action="run" gtfTarget=gtfDir {
			thread.paths = [];
			for (var i = 1; i <= 300; i++) {
				thread.paths.append(getTempFile(attributes.gtfTarget, "gtf"));
			}
		}
	}
	thread action="join" name="#gtfThreads#";
	gtfSeen = {};
	gtfCalls = 0;
	for (gtfName in listToArray(gtfThreads)) {
		for (gtfPath in cfthread[gtfName].paths) {
			gtfCalls++;
			gtfSeen[gtfPath] = true;
		}
	}
	assert("1800 calls from 6 threads at once", gtfCalls, 1800);
	assert("give 1800 different files", structCount(gtfSeen), 1800);
	assertFalse("none of them is a path the loop was given", structKeyExists(gtfSeen, structKeyList(gtfLoop).listFirst()));

	assert("every path handed out is a file on disk", arrayLen(directoryList(gtfDir, false, "name")), 1 + 500 + 1800);

	assertThrows("a directory that does not exist is an error", function() {
		getTempFile(gtfDir & "/missing", "gtf");
	});
} finally {
	try { directoryDelete(gtfDir, true); } catch (any e) {}
}

suiteEnd();
</cfscript>
