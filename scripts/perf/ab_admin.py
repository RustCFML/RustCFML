#!/usr/bin/env python3
"""Interleaved ABBA A/B of two rustcfml binaries on AUTHENTICATED Preside admin
pages — the workload the PGO profile had never seen.

Measures SERVER CPU SECONDS (the box is busy; wall clock lies). Each leg boots a
fresh server, logs in with a real form POST (browser-ish headers are REQUIRED —
without an Accept header Preside answers 401 instead of 302 to the login form),
discards boot, then times a fixed number of warm admin renders spread across
several admin pages.

Shutdown is SIGINT, never SIGTERM: the engine only handles ctrl_c(), and a
SIGTERM'd Preside leaves a startup lock held so the NEXT leg boots forever.
"""
import subprocess, time, os, statistics, signal, urllib.request, urllib.parse, http.cookiejar, re, sys

SITE = os.environ.get("SITE", "/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website")
SCRATCH = os.path.dirname(os.path.abspath(__file__))
RENDERS = int(os.environ.get("RENDERS", "60"))
LEGS = os.environ.get("LEGS", "ABBAABBAABBA")
UA = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/120 Safari/537.36")
ADMIN_PAGES = [
    "/admin/", "/admin/sitetree/", "/admin/datamanager/", "/admin/dashboard/",
    "/admin/assetmanager/", "/admin/datamanager/object/?id=system_alert",
    "/admin/sitetree/trash/", "/admin/editprofile/", "/admin/auditTrail/",
    "/admin/youtube/",
]

BIN = {"A": os.environ.get("BIN_A"), "B": os.environ.get("BIN_B")}
NAME = {"A": os.environ.get("NAME_A", "A"), "B": os.environ.get("NAME_B", "B")}


def cpu_seconds(pid):
    out = subprocess.run(["ps", "-o", "time=", "-p", str(pid)],
                         capture_output=True, text=True).stdout.strip()
    if not out:
        return None
    parts = [float(p) for p in out.split(":")]
    while len(parts) < 3:
        parts.insert(0, 0.0)
    return parts[0] * 3600 + parts[1] * 60 + parts[2]


def opener():
    cj = http.cookiejar.CookieJar()
    op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cj))
    op.addheaders = [("User-Agent", UA),
                     ("Accept", "text/html,application/xhtml+xml")]
    return op


def login(op, port):
    with op.open(f"http://127.0.0.1:{port}/admin/", timeout=600) as r:
        html = r.read().decode("utf-8", "replace")
    m = re.search(r'<form[^>]*action="([^"]*login/login/[^"]*)"', html)
    if not m:
        raise RuntimeError("login form not found — did the site answer 401?")
    data = urllib.parse.urlencode({"loginId": "sysadmin", "password": "password"}).encode()
    with op.open(m.group(1), data=data, timeout=600) as r:
        body = r.read().decode("utf-8", "replace")
    if "You do not have access" in body:
        raise RuntimeError("login rejected")
    return True


def leg(binary, port, tag):
    log = open(f"{SCRATCH}/abadm_{tag}.log", "w")
    p = subprocess.Popen([binary, "--serve", SITE, "--port", str(port), "--production"],
                         stdout=log, stderr=subprocess.STDOUT)
    try:
        op = opener()
        for _ in range(600):
            time.sleep(1)
            try:
                op.open(f"http://127.0.0.1:{port}/?cb=boot", timeout=600).read()
                break
            except Exception:
                if p.poll() is not None:
                    raise RuntimeError(f"{tag}: server DIED during boot")
        else:
            raise RuntimeError(f"{tag}: server never answered")
        login(op, port)
        for pg in ADMIN_PAGES:                      # warm every page once
            op.open(f"http://127.0.0.1:{port}{pg}", timeout=600).read()

        t0 = cpu_seconds(p.pid)
        w0 = time.time()
        for i in range(RENDERS):
            pg = ADMIN_PAGES[i % len(ADMIN_PAGES)]
            sep = "&" if "?" in pg else "?"
            op.open(f"http://127.0.0.1:{port}{pg}{sep}cb={tag}{i}", timeout=600).read()
        wall = time.time() - w0
        t1 = cpu_seconds(p.pid)
        return (t1 - t0) / RENDERS * 1000.0, wall / RENDERS * 1000.0
    finally:
        p.send_signal(signal.SIGINT)
        try:
            p.wait(timeout=180)
        except subprocess.TimeoutExpired:
            p.kill()
        log.close()


res = {"A": [], "B": []}
port = int(os.environ.get("PORT0", "9800"))
for i, side in enumerate(LEGS):
    port += 1
    cpu, wall = leg(BIN[side], port, f"{side}{i}")
    res[side].append(cpu)
    print(f"leg {i+1} [{side}] {NAME[side]:>24}: {cpu:7.3f} ms CPU/render ({wall:7.3f} ms wall)",
          flush=True)

print()
for s in "AB":
    v = res[s]
    print(f"{NAME[s]:>24}: mean {statistics.mean(v):7.3f}  best {min(v):7.3f}  "
          f"legs {[round(x,3) for x in v]}")
ma, mb = statistics.mean(res["A"]), statistics.mean(res["B"])
pairs = []
flat = [(s, v) for s in LEGS for v in []]
seq = []
idx = {"A": 0, "B": 0}
for s in LEGS:
    seq.append((s, res[s][idx[s]])); idx[s] += 1
for i in range(len(seq) - 1):
    (s1, v1), (s2, v2) = seq[i], seq[i + 1]
    if s1 != s2:
        a, b = (v1, v2) if s1 == "A" else (v2, v1)
        pairs.append((b - a) / a * 100)
spread = max(res["A"]) - min(res["A"])
print(f"\ndelta (mean): {(mb-ma)/ma*100:+.2f}%   best-case: "
      f"{(min(res['B'])-min(res['A']))/min(res['A'])*100:+.2f}%")
if pairs:
    print(f"adjacent-pair median: {statistics.median(pairs):+.2f}%   pairs={[round(p,2) for p in pairs]}")
print(f"A-to-A spread: {spread:.3f} ms ({spread/ma*100:.2f}%) — a delta below this is not a result")
