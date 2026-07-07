<!---
  GitHub #250: every cfcatch object must carry the full standard member set
  (type, message, detail, errorCode, extendedInfo, tagContext, stackTrace) with
  empty string for unset members — never absent — regardless of how the
  exception originated. throw()-created exceptions already carried them, but
  engine-raised (native) errors omitted errorCode and extendedInfo. Since v0.408
  ("undefined member reads throw") a handler reading e.extendedInfo unguarded
  then threw a SECONDARY "Variable 'extendedInfo' is undefined" that replaced
  the real error — exactly what TestBox's failure recorder does, aborting every
  sibling spec in the bundle.

  Fix: default errorCode/extendedInfo (empty string) centrally in add_root_cause,
  alongside the existing stackTrace default, so every exception struct carries
  the full contract; a value explicitly set by throw() is preserved.
--->
<cfscript>
suiteBegin("cfcatch standard member set on all error origins (GitHub 250)");

// The members every JVM engine guarantees.
members = ["type","message","detail","errorCode","extendedInfo","tagContext","stackTrace"];

// 1. throw()-created exception has the full set.
try {
    throw(type="TestType", message="boom");
} catch (any e) {
    for (m in members) {
        assert("throw() cfcatch has member " & m, structKeyExists(e, m), true);
    }
}

// 2. Native engine-raised error (undefined variable) has the full set —
//    this was the gap: errorCode/extendedInfo were missing.
try {
    x = totallyUndefinedVariable_250;
} catch (any e) {
    for (m in members) {
        assert("native-error cfcatch has member " & m, structKeyExists(e, m), true);
    }
    assert("native errorCode is empty string", e.errorCode, "");
    assert("native extendedInfo is empty string", e.extendedInfo, "");
}

// 3. Native undefined struct-member read (another common native shape).
try {
    s = {};
    y = s.missingKey_250;
} catch (any e) {
    assert("undefined-member cfcatch has errorCode", structKeyExists(e, "errorCode"), true);
    assert("undefined-member cfcatch has extendedInfo", structKeyExists(e, "extendedInfo"), true);
}

// 4. The TestBox failure-recorder shape: reading extendedInfo/errorCode/stackTrace
//    unguarded on a native error must NOT throw a secondary error.
recorderThrew = false;
try {
    try {
        z = alsoUndefined_250;
    } catch (any err) {
        recorded = err.extendedInfo & err.errorCode & err.stackTrace;
    }
} catch (any outer) {
    recorderThrew = true;
}
assert("unguarded recorder read of native error does not secondary-throw", recorderThrew, false);

// 5. throw()'s explicit errorCode/extendedInfo values are preserved (not clobbered
//    to empty by the central defaulting).
try {
    throw(type="T", message="m", errorcode="E-42", extendedinfo="extra-info");
} catch (any e) {
    assert("throw() errorCode preserved", e.errorCode, "E-42");
    assert("throw() extendedInfo preserved", e.extendedInfo, "extra-info");
}

suiteEnd();
</cfscript>
