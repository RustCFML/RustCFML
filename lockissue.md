# Boot-window `cflock` rejection — requests arriving during app startup 500 instead of waiting

**Status:** OPEN, reproducible, not fixed. Written up 2026-07-29 against RustCFML v0.532.0.

**Not** to be confused with the concurrency/function-ID race that produces
`Variable is not a function or function '_targetAction' is not defined` — that is a separate
bug with a separate root cause.

## Symptom

Any CFML request that arrives while an application is still booting fails immediately with:

```
Runtime Error: cflock timeout: could not acquire exclusive lock within 0ms
  1: onError (…/preside/system/Bootstrap.cfc:143)
  2: (main)  (…/preside/system/Bootstrap.cfc:244)
```

HTTP 500, returned in ~5–22 ms — i.e. it does not wait, it is refused instantly. Preside's
boot takes ~10–12 s on the Ready Intelligence site, so the window is wide.

## Reproduction

Cold-start the server, then fire several requests concurrently. One request drives the boot and
succeeds; the rest are rejected.

```
6 parallel authenticated admin requests at cold boot:
  req1: HTTP 500 8285b in 0.023s      req4: HTTP 500 8285b in 0.022s
  req2: HTTP 500 8285b in 0.023s      req5: HTTP 500 8285b in 0.022s
  req3: HTTP 500 8285b in 0.023s      req6: HTTP 200 193225b in 13.104s
```

A **single** request during boot always succeeds, because it is the one performing the boot.
The bug needs concurrency, which is why it is invisible to scripted sequential testing and
completely reliable from a real browser.

Follow-up requests after boot completes recover normally — there is no lasting poisoning of
application state.

## Second face of the same bug: broken CSS/JS on every cold start

The site's `urlrewrite.xml` routes every static asset through the CFML pipeline:

```xml
<rule>
    <from>^/preside/system/assets/.*$</from>
    <to last="true">%{context-path}/index.cfm</to>
</rule>
```

So stylesheets and scripts are CFML requests, even though the files exist on disk. During boot
they are rejected like any other request and return an **HTML error page**, which the browser
refuses:

> Did not parse stylesheet at '…/_5867a4c6.dialog.min.css' because non CSS MIME types are not
> allowed in strict mode.

Verified by racing asset requests against a boot on a scratch port:

```
css1 : code=500 type=text/html; charset=utf-8 time=0.005s
css2 : code=500 type=text/html; charset=utf-8 time=0.005s
css3 : code=500 type=text/html; charset=utf-8 time=0.005s
page : code=200 type=text/html;charset=UTF-8
```

The body of each is the RustCFML debug error page carrying the `cflock timeout` message. The
same URL requested alone, or after boot, returns `200 content-type: text/css`.

### Caching: not a factor (checked)

Headers for the same URL, same server instance:

| | during boot | after boot |
|---|---|---|
| status | 500 | 200 |
| content-type | `text/html; charset=utf-8` | `text/css` |
| cache-control | *(absent)* | `max-age=31536000` |
| etag | *(absent)* | `227ac611` |

The 500 carries no `cache-control` and no `etag`, and a 500 is not cacheable by default under
HTTP semantics, so nothing is being asked to store it. It is not sticky server-side either:
on the same instance the next request after boot returned `200 text/css`. What looks like
caching is browser behaviour — a stylesheet that failed is not re-requested for the life of
the page, and reloading during the boot window simply fails again.

Worth noting for any future fix: the *successful* asset response is `max-age=31536000` on a
content-hashed URL. That is correct for immutable assets, but it means error responses on
asset paths must never inherit the asset cache policy, or a poisoned entry would persist for
a year.

## Suspicious detail

The message says `within 0ms`. A zero-length timeout means a request contending for the
startup lock fails instantly rather than waiting for boot to finish. Whether the `0ms` comes
from Preside's `<cflock>` attributes or from how RustCFML interprets/defaults the timeout has
**not** been established and is the first thing to check.

## Explicitly unverified

An earlier claim in conversation that "Lucee queues instead of erroring" was asserted without
testing and was challenged by Alex, who believes Lucee also errors on a zero-wait lock rather
than queueing. **Treat the Lucee comparison as unknown.** Before designing a fix, verify the
actual reference behaviour (`box server start cfengine=lucee@7`, never `@be`) — Lucee is the
canonical target, so the fix should match whatever it genuinely does, which may well be
"error, but with a startup-specific message" rather than "wait".

## Fix directions (undecided)

1. Establish where the `0ms` timeout originates; if RustCFML is defaulting a missing/blank
   timeout to 0 where Lucee would use a real value, that alone may be the bug.
2. If erroring is correct reference behaviour, make it a *recognisable* startup condition —
   e.g. HTTP 503 with `Retry-After` and a plain-text/CSS-safe body per content type — rather
   than a 500 HTML debug page delivered in place of a stylesheet.
3. Consider holding non-boot requests until boot completes (queue behind the startup lock),
   but only if that matches Lucee.

## Reproduction notes

Preside binds the admin session to the **User-Agent**: replaying the `psid` cookie alone
returns a 401 login page. Send the browser UA string too, plus cookies `psid`, `jsessionid`,
`cfid`, `cftoken`, `_presideEditMode`, `DefaultLocale`. Find a live session with:

```sql
select id, from_unixtime(expiry) from psys_session_storage where value like '%admin_user%'
order by expiry desc;
```

DB credentials are in the site's `.cfconfig.json` (datasource `preside`, database
`pcms_ritest`).
