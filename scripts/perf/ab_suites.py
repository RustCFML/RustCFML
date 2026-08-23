#!/usr/bin/env python3
"""Interleaved ABBA A/B of two rustcfml binaries across LONG, REAL workloads.

Why this exists
---------------
Warm single-page A/Bs and microbenchmarks have repeatedly failed to see real
effects on this engine: a warm Preside homepage is ~20 ms of which a large slice
is fixed per-request cost, so a 2% engine change sits under the noise. The
workloads here each run for tens of seconds to minutes of almost pure CFML
execution, which is where an interpreter change actually shows up.

  own      RustCFML's own suite (CLI, no server)      ~10 s
  testbox  TestBox's own suite over HTTP              ~30 s
  wheels   Wheels core suite over HTTP                ~170 s
  preside  Preside-CMS TestBox unit suite (quick)     ~120 s
  traffic  UNCACHED Preside page traffic, ?cb= each   ~60 s
  procfd   frame-dense PROCEDURAL CFML, CLI, no DB    ~20 s

⚠️ `wheels` and `testbox` are NOT instruments for per-frame engine levers.
Measured 2026-08-23: Wheels spends ~35 us of CPU per frame and TestBox ~62 us,
against Preside's ~4 us -- the rest is SQL, ORM and IO. The v0.616 default-params
lever reaches 16% of Wheels frames and 17% of TestBox frames (21% on Preside), so
its REACH is the same everywhere, but dilution puts its predicted effect at 0.18%
on Wheels versus 2-4% on Preside. It measured -3.87% on Preside, nothing on
Wheels, and -27% on `procfd`. A per-frame lever that "does nothing on Wheels" has
almost certainly just been diluted; check frames-per-CPU-second before concluding
anything. `procfd` exists so the standing "keep a second workload" rule is served
by a workload that can actually SEE a frame lever.

Metric is SERVER CPU SECONDS, not wall clock (Rule 4: CPU is valid for
RustCFML-vs-RustCFML; it is NOT valid cross-engine). Wall is reported too, but
only as a sanity check.

Every leg records a SANITY string (pass counts / body size). A leg that got
faster because it did less work is the single most common way this measurement
lies -- an expired session, a boot failure, a 302. Legs whose sanity string
differs from the first leg's are REFUSED, not averaged.

  BIN_A=/path/a BIN_B=/path/b LEGS=ABBA WORKLOADS=own,testbox,traffic \
      python3 scripts/perf/ab_suites.py

Shutdown is SIGINT, never SIGTERM -- a SIGTERM'd Preside holds the startup lock
and every later leg boots forever (one bad leg poisons the whole run).
"""
import http.client, os, re, signal, socket, statistics, subprocess, sys, time, urllib.request

http.client._MAXHEADERS = 1000   # Wheels' reporter exceeds urllib's default of 100

REPO     = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRATCH  = os.environ.get("SCRATCH", "/tmp")
SITE     = os.environ.get("SITE",     "/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website")
WHEELS   = os.environ.get("WHEELS",   "/Users/alexskinner/Repos/opensource/CFMLs/wheels/public")
TESTBOX  = os.environ.get("TESTBOX",  "/Users/alexskinner/Repos/opensource/CFMLs/TestBox")
PRESIDET = os.environ.get("PRESIDET", "/Users/alexskinner/Repos/opensource/Preside-CMS/tests")
WCFG     = os.environ.get("WCFG",     "/private/tmp/claude-501/-Users-alexskinner-Repos-opensource-CFMLs-RustCFML/4791baff-70fe-43d7-99f9-08ccda79d2c8/scratchpad/wheels.cfconfig.json")
PCFG     = os.environ.get("PCFG",     "/private/tmp/claude-501/-Users-alexskinner-Repos-opensource-CFMLs-RustCFML/4791baff-70fe-43d7-99f9-08ccda79d2c8/scratchpad/preside_test.cfconfig.json")
PORT     = int(os.environ.get("PORT", "8611"))
RENDERS  = int(os.environ.get("RENDERS", "300"))
REPEATS  = int(os.environ.get("REPEATS", "4"))   # suite runs per leg — see note above
LEGS     = os.environ.get("LEGS", "ABBA")
import json as _json
ENV_A    = _json.loads(os.environ.get("ENV_A", "{}"))
ENV_B    = _json.loads(os.environ.get("ENV_B", "{}"))
ARM_ENV  = {}   # set per leg in main()


def footer_off_cfconfig(path):
    """Return a copy of `path` with debugging disabled, written into SCRATCH.

    Rule 7 applies to every workload, not just the site: the Preside test cfconfig
    sets `debugging.enabled: true`, which appends the debug footer (environment dump
    plus a timing row per executed file) to the suite's own output. The footer is
    rendered CFML, so it inflates the CPU the harness is trying to attribute."""
    import json
    if not os.path.exists(path):
        return path
    cfg = json.load(open(path))
    if isinstance(cfg.get("debugging"), dict):
        cfg["debugging"]["enabled"] = False
    else:
        cfg["debugging"] = {"enabled": False}
    out = os.path.join(SCRATCH, "ab_suites_nofooter_" + os.path.basename(path))
    open(out, "w").write(json.dumps(cfg, indent=2))
    return out


def cpu_seconds(pid):
    out = subprocess.run(["ps", "-o", "time=", "-p", str(pid)],
                         capture_output=True, text=True).stdout.strip()
    if not out:
        return None
    parts = [float(p) for p in out.split(":")]
    while len(parts) < 3:
        parts.insert(0, 0.0)
    return parts[0] * 3600 + parts[1] * 60 + parts[2]


def wait_up(port, path, timeout=300):
    """TCP-connect probe, not an HTTP 200.

    TestBox's webroot has no index page, so a 200-probe called a perfectly healthy
    server dead. `path` is kept only for the workloads that want a real warm-up
    request (Preside must finish booting before the clock starts).
    """
    import socket
    end = time.time() + timeout
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=5):
                pass
        except OSError:
            time.sleep(1)
            continue
        if path is None:
            return True
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=120):
                return True
        except Exception:
            time.sleep(1)
    return False


def serve(binary, root, port, extra=(), env=None):
    e = dict(os.environ)
    if env:
        e.update(env)
    e.update(ARM_ENV)          # arm env wins: it is the thing under test
    log = open(f"{SCRATCH}/ab_suites_{port}.log", "w")
    p = subprocess.Popen([binary, "--serve", root, "--port", str(port), *extra],
                         stdout=log, stderr=subprocess.STDOUT, env=e)
    return p


def stop(p):
    try:
        p.send_signal(signal.SIGINT)
        p.wait(timeout=45)
    except Exception:
        try:
            p.kill()
        except Exception:
            pass


def get(port, path, timeout=900):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=timeout) as r:
        return r.read().decode("utf-8", "replace")


# ---------------------------------------------------------------- workloads
# Each returns (cpu_seconds, wall_seconds, sanity_string).

def wl_own(binary):
    t0 = time.monotonic()
    before = os.times()
    _e = dict(os.environ); _e.update(ARM_ENV)
    out = subprocess.run([binary, f"{REPO}/tests/runner.cfm"],
                         capture_output=True, text=True, cwd=REPO, env=_e).stdout
    after = os.times()
    wall = time.monotonic() - t0
    cpu = (after.children_user - before.children_user) + (after.children_system - before.children_system)
    m = re.search(r"SUMMARY: (\d+)/(\d+) passed across (\d+) suites", out)
    return cpu, wall, (m.group(0) if m else "NO-SUMMARY")


def _http_suite(binary, root, up_path, run_path, extra=(), env=None, sanity=None, port=None):
    port = port or PORT
    p = serve(binary, root, port, extra, env)
    try:
        if not wait_up(port, up_path):
            return None, None, "SERVER-DID-NOT-START"
        get(port, run_path)                       # discard: first pass pays compile+cache fill
        reps = 1 if ("/index.cfm/wheels" in run_path or "runtests.cfm" in run_path) else REPEATS
        c0, t0 = cpu_seconds(p.pid), time.monotonic()
        body = None
        for _ in range(reps):
            body = get(port, run_path)
        wall, c1 = time.monotonic() - t0, cpu_seconds(p.pid)
        return c1 - c0, wall, sanity(body)
    finally:
        stop(p)


def wl_testbox(binary):
    def s(b):
        m = re.search(r'"totalPass"\s*:\s*(\d+).*?"totalFail"\s*:\s*(\d+).*?"totalError"\s*:\s*(\d+)', b, re.S)
        return f"pass={m.group(1)} fail={m.group(2)} err={m.group(3)}" if m else f"bytes={len(b)}"
    return _http_suite(binary, TESTBOX, None, "/tests/runner.cfm?reporter=json", sanity=s)


def wl_procfd(binary):
    """Frame-dense PROCEDURAL CFML: plain page-level UDFs, defaulted params, no
    DB, no IO, no CFCs. The workload Wheels would be if it were not database
    bound -- see the dilution note in the module docstring."""
    src = os.path.join(SCRATCH, "ab_procfd.cfm")
    with open(src, "w") as f:
        f.write(
            "<cfscript>\n"
            "function withDefaults( required numeric a, numeric b = 2, string c = \"x\" ) {\n"
            "    return a + b + len( c );\n"
            "}\n"
            "function plain( a, b ) { return a + b; }\n"
            "total = 0;\n"
            "for ( i = 1; i <= 300000; i++ ) { total += withDefaults( i ) + plain( i, 1 ); }\n"
            "writeOutput( total );\n"
            "</cfscript>\n"
        )
    _e = dict(os.environ)
    _e.update(ARM_ENV)
    t0 = time.monotonic()
    before = os.times()
    out = subprocess.run([binary, src], capture_output=True, text=True, env=_e).stdout
    after = os.times()
    wall = time.monotonic() - t0
    cpu = ((after.children_user - before.children_user)
           + (after.children_system - before.children_system))
    # Sanity: the arithmetic result. A run that errored early is fast and wrong.
    return cpu, wall, f"sum={out.strip()[-14:]}"


def wl_wheels(binary):
    """Wheels core suite on sqlite.

    Needs its datasource: `wheels/lucee.json` declares `wheelstestdb_sqlite` in the
    Lucee/JDBC form (`dbdriver: "Other"`, `class: org.sqlite.JDBC`), which RustCFML
    does not map — it wants `dbdriver: "sqlite"` with `database` = the file path.
    Without that every request is a 500 in 0.1 s, which looks blazingly fast.
    """
    def s(b):
        p_ = len(re.findall(r'"status"\s*:\s*"Passed"', b))
        sk = len(re.findall(r'"status"\s*:\s*"Skipped"', b))
        f_ = sum(int(x) for x in re.findall(r'"totalFail"\s*:\s*"?(\d+)"?', b))
        e_ = sum(int(x) for x in re.findall(r'"totalError"\s*:\s*"?(\d+)"?', b))
        return f"pass={p_} skip={sk} fail={f_} err={e_}"
    return _http_suite(binary, WHEELS, None,
                       "/index.cfm/wheels/core/tests?db=sqlite&reload=true&format=json&cli=true",
                       extra=("--cfconfig", WCFG), sanity=s, port=PORT + 2)


def wl_preside(binary):
    """Preside-CMS's own TestBox unit suite, quick scope. Baseline 1560/13/50/2."""
    def s(b):
        # LAST match, not the first: every bundle prints its own "[Passed: n]" line,
        # so re.search() returned a per-bundle count (pass=2) that was identical on
        # every leg -- a sanity check that could never fail is worse than none.
        ms = re.findall(r"\[Passed:\s*(\d+)\]\s*\[Failed:\s*(\d+)\]\s*"
                        r"\[Errors:\s*(\d+)\]\s*\[Skipped:\s*(\d+)\]", b)
        if not ms:
            return f"NO-STATS bytes={len(b)}"
        p_, f_, e_, sk = ms[-1]
        return f"pass={p_} fail={f_} err={e_} skip={sk}"
    return _http_suite(binary, PRESIDET, None, "/runtests.cfm?reporter=text&scope=quick",
                       extra=("--cfconfig", footer_off_cfconfig(PCFG)),
                       env={"PRESIDETEST_DB_HOST": "localhost", "PRESIDETEST_DB_PORT": "3306",
                            "PRESIDETEST_DB_NAME": "preside_test", "PRESIDETEST_DB_USER": "root",
                            "PRESIDETEST_DB_PASSWORD": "freeze"},
                       sanity=s, port=PORT + 3)


def wl_traffic(binary):
    """UNCACHED Preside page traffic: a fresh ?cb= per request so nothing is served
    from a content cache. --production so the ENGINE caches match deployment; the
    point is to defeat Preside's caches, not the bytecode cache."""
    p = serve(binary, SITE, PORT + 4, ("--production",))
    try:
        if not wait_up(PORT + 4, "/?cb=boot", timeout=300):
            return None, None, "SERVER-DID-NOT-START"
        for i in range(5):                      # discard warm-up
            get(PORT + 4, f"/?cb=w{i}")
        c0, t0, total = cpu_seconds(p.pid), time.monotonic(), 0
        for i in range(RENDERS):
            total += len(get(PORT + 4, f"/?cb=r{i}"))
        wall, c1 = time.monotonic() - t0, cpu_seconds(p.pid)
        return c1 - c0, wall, f"renders={RENDERS} meanbytes={total // RENDERS}"
    finally:
        stop(p)


class FooterOff:
    """Rule 7: debug footer OFF while measuring, ALWAYS restored.

    The site's `.cfconfig.json` sets `debugging.enabled: true` (worth ~1.18 ms/render).
    The footer times every executed file and is itself rendered CFML, so leaving it on
    both inflates the measurement and changes what is being measured. Restored in
    __exit__ so an exception or a Ctrl-C cannot leave the user's site modified."""

    def __init__(self, site):
        self.path = os.path.join(site, ".cfconfig.json")
        self.backup = None

    def __enter__(self):
        if not os.path.exists(self.path):
            return self
        import json
        self.backup = open(self.path).read()
        cfg = json.loads(self.backup)
        if isinstance(cfg.get("debugging"), dict):
            cfg["debugging"]["enabled"] = False
        else:
            cfg["debugging"] = {"enabled": False}
        open(self.path, "w").write(json.dumps(cfg, indent=2))
        return self

    def __exit__(self, *exc):
        if self.backup is not None:
            open(self.path, "w").write(self.backup)
        return False


ALL = {"own": wl_own, "testbox": wl_testbox, "wheels": wl_wheels,
       "preside": wl_preside, "traffic": wl_traffic, "procfd": wl_procfd}


def permutation_p(A, B):
    """Exact two-sided permutation test on the difference of means.

    Legs are exchangeable under the null (same binary, same box), so enumerate every
    way of splitting the pooled legs into arms of the observed sizes and ask how often
    |difference| is at least as large as observed. Exact while the count is small;
    sampled beyond that."""
    import itertools, random
    pool = A + B
    na = len(A)
    obs = abs(statistics.mean(B) - statistics.mean(A))
    idx = range(len(pool))
    combos = list(itertools.combinations(idx, na))
    if len(combos) > 20000:
        combos = [tuple(random.sample(list(idx), na)) for _ in range(20000)]
    hits = 0
    for c in combos:
        cs = set(c)
        a = [pool[i] for i in cs]
        b = [pool[i] for i in idx if i not in cs]
        if abs(statistics.mean(b) - statistics.mean(a)) >= obs - 1e-12:
            hits += 1
    return hits / len(combos)


def main():
    a, b = os.environ.get("BIN_A"), os.environ.get("BIN_B")
    if not a or not b:
        sys.exit("set BIN_A and BIN_B")
    names = [w.strip() for w in os.environ.get("WORKLOADS", "own,testbox,traffic").split(",") if w.strip()]
    for n in names:
        if n not in ALL:
            sys.exit(f"unknown workload {n}; have {list(ALL)}")

    la = os.getloadavg()[0]
    if la > 5.0:
        print(f"WARNING: 1-min load average {la:.2f} > 5.0 — a busy box has invalidated "
              f"an entire A/B run before. Treat the spread, not the delta, as the story.",
              flush=True)
    res = {n: {"A": [], "B": []} for n in names}
    sane = {}
    footer = FooterOff(SITE) if "traffic" in names else None
    if footer:
        footer.__enter__()
        print("debug footer disabled for the run (restored on exit)", flush=True)
    try:
      for i, arm in enumerate(LEGS):
          binary = a if arm == "A" else b
          for n in names:
              cpu, wall, s = ALL[n](binary)
              if cpu is None:
                  print(f"  leg{i} {arm} {n:8s} FAILED: {s}", flush=True)
                  continue
              key = f"{n}"
              if key not in sane:
                  sane[key] = s
              flag = "" if s == sane[key] else f"  !! SANITY DRIFT (first leg: {sane[key]})"
              print(f"  leg{i} {arm} {n:8s} cpu={cpu:7.2f}s wall={wall:7.2f}s  {s}{flag}", flush=True)
              if flag:
                  continue                        # refuse: it did different work
              res[n][arm].append(cpu)
    finally:
        if footer:
            footer.__exit__()
            print("debug footer restored", flush=True)

    print("\n=== RESULT (server CPU seconds; A = BIN_A, B = BIN_B) ===")
    print(f"{'workload':10s} {'A mean':>8s} {'B mean':>8s} {'delta':>8s} {'delta%':>8s} "
          f"{'A sd':>6s} {'B sd':>6s} {'p':>7s}  verdict")
    for n in names:
        A, B = res[n]["A"], res[n]["B"]
        if len(A) < 2 or len(B) < 2:
            print(f"{n:10s} insufficient legs (A={len(A)} B={len(B)}) — need >=2 each")
            continue
        am, bm = statistics.mean(A), statistics.mean(B)
        asd = statistics.stdev(A)
        bsd = statistics.stdev(B)
        d = bm - am
        pct = 100 * d / am
        p_val = permutation_p(A, B)
        # Two independent bars. The null calibration (identical binaries, 4 legs) showed
        # apparent deltas up to -3.77% and floors of 3.8-8.8%, so "the means differ" is
        # not evidence of anything on its own.
        if p_val > 0.05:
            verdict = f"NOT A RESULT (p={p_val:.3f} > 0.05)"
        elif abs(d) < max(asd, bsd):
            verdict = "NOT A RESULT (delta < within-arm sd)"
        else:
            verdict = ("B faster" if d < 0 else "B slower") + f" — significant"
        print(f"{n:10s} {am:8.2f} {bm:8.2f} {d:+8.2f} {pct:+7.2f}% "
              f"{asd:6.2f} {bsd:6.2f} {p_val:7.3f}  {verdict}")
    print("\nNull calibration at 4 legs (identical binaries) gave floors of 3.8-8.8% and an\n"
          "apparent -3.77% on preside. Treat anything under ~4% with fewer than ~12 legs\n"
          "per arm as unresolved, whatever the means say.")


if __name__ == "__main__":
    main()
