<!---
  GitHub #245: java.io.File shim write-side methods were silent no-ops.
  mkdirs()/mkdir() returned null and created nothing; the static field
  File.separator was missing (read threw "Variable 'separator' is undefined").
  Wheels uses CreateObject("java","java.io.File").init(path).mkdirs() as its
  cross-engine recursive-mkdir idiom.

  Fix: mkdirs()->create_dir_all, mkdir()->create_dir, delete()/createNewFile()
  wired to std::fs, all returning JDK boolean semantics (true if created/removed,
  false on failure/already-exists, no throw). separator/separatorChar/
  pathSeparator/pathSeparatorChar exposed as static fields on the shim.
--->
<cfscript>
suiteBegin("java.io.File mkdirs/mkdir/separator (GitHub 245)");

base = GetTempDirectory() & "rustcfml-file-245-" & CreateUUID();
DirectoryCreate(base);

// mkdirs() creates all missing parents, returns true.
nested = base & "/w/x/y";
g = CreateObject("java", "java.io.File").init(nested);
assert("mkdirs returns true on create", g.mkdirs(), true);
assert("mkdirs actually created the nested path", DirectoryExists(nested), true);

// mkdirs() on an existing directory returns false (JDK contract).
assert("mkdirs returns false when dir already exists", g.mkdirs(), false);

// mkdir() creates a single level whose parent exists.
single = base & "/one";
h = CreateObject("java", "java.io.File").init(single);
assert("mkdir returns true on create", h.mkdir(), true);
assert("mkdir actually created the dir", DirectoryExists(single), true);

// FileWrite into a mkdirs-created path now succeeds.
FileWrite(nested & "/a.txt", "hi");
assert("FileWrite into mkdirs path works", FileRead(nested & "/a.txt"), "hi");

// createNewFile / delete boolean contract.
nf = base & "/new.txt";
fnf = CreateObject("java", "java.io.File").init(nf);
assert("createNewFile returns true", fnf.createNewFile(), true);
assert("createNewFile returns false when it exists", fnf.createNewFile(), false);
assert("delete returns true", fnf.delete(), true);
assert("delete actually removed the file", FileExists(nf), false);

// Static File.separator (accessed WITHOUT init, and on an instance).
assert("File.separator (no init) is single char", len(CreateObject("java", "java.io.File").separator), 1);
finst = CreateObject("java", "java.io.File").init(base);
assert("f.separator present", len(finst.separator) >= 1, true);
assert("f.pathSeparator present", len(finst.pathSeparator) >= 1, true);

// The shim variable is NOT clobbered by a mkdirs()/delete() call (parity with
// the java.lang.System setProperty receiver-clobber family, GitHub #249).
probe = CreateObject("java", "java.io.File").init(base & "/probe");
r = probe.mkdirs();
assert("File shim not nulled after mkdirs", isNull(probe), false);
assert("File shim still usable after mkdirs", probe.exists(), true);

if (DirectoryExists(base)) DirectoryDelete(base, true);

suiteEnd();
</cfscript>
