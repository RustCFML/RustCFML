# Preside — Java surface area (what core Preside consumes, and how)

> Companion to **[`PRESIDE_BOOT_JAVA_NOOPS.md`](PRESIDE_BOOT_JAVA_NOOPS.md)** and
> **[`docs/java-shims.md`](docs/java-shims.md)**.
>
> - `docs/java-shims.md` — what the **engine provides**.
> - `PRESIDE_BOOT_JAVA_NOOPS.md` — the five deps that **blocked boot**, and the
>   temporary no-ops applied to get past them.
> - **This file** — every Java class core Preside **consumes**, what *surface* of
>   each it actually touches, and what a shim would need to satisfy. Scoped to the
>   whole app, not just the boot path.
>
> Source tree: `/Users/alexskinner/Repos/opensource/Preside-CMS` @ `chrono-port`.
> Survey excludes `system/externals/**` (vendored: coldbox, sticker, cfconcurrent,
> chrono, lucee-spreadsheet) and `tests/testbox/**`.

## Regenerating

The literal-classname inventory:

```sh
cd /Users/alexskinner/Repos/opensource/Preside-CMS
grep -rnE "createobject\(\s*[\"']java[\"']" --include="*.cfc" --include="*.cfm" -i system/ \
  | grep -v "/externals/"
```

⚠️ That misses **dynamic classnames** — three call sites build the class string at
runtime and will not appear:

| Helper | File | Classes reached |
|---|---|---|
| `_getPlantUmlObj( className )` | `system/services/cfflow/util/PlantUmlDiagramService.cfc:29` | `net.sourceforge.plantuml.*` |
| `_new( className )` | `system/services/email/EmailLoggingService.cfc:1018` | `org.jsoup.Jsoup` |
| `_new( className )` | `system/services/email/EmailStyleInliner.cfc:166` | `org.jsoup.Jsoup` |

Cross-reference the result against `docs/java-shims.md`. Two distinct failure modes
matter here (`docs/java-shims.md:28-29`):

- **Unsupported *class*** → `createObject` **throws** loudly at construction
  (`Java class [x] is not supported`). Fails fast, easy to spot.
- **Unsupported *method* on a supported class** → usually returns **`null`
  silently**, so the failure surfaces somewhere confusing downstream.

"Class is on the shim list" is therefore not sufficient; the per-entry **Surface
used** field below exists so a shim author can check method-level coverage.

## Legend

- 🛑 **Blocks the feature** — class not shimmed; `createObject` throws. Feature dead, but loudly.
- 🏗 **Partial** — class shimmed, specific members used here unverified. **Silent-`null` risk.**
- ✅ **Covered** — class and members present in `docs/java-shims.md`.
- 🥇 **Cheap win** — removable from Preside outright, no engine work needed.

---

# Part 1 — Gaps (unshimmed, ordered by cost-to-fix)

## 1. `java.awt.image.BufferedImage` 🥇

- **Call site:** `system/services/assetManager/NativeImageService.cfc:258`
- **Feature:** PDF → JPG thumbnail on asset upload.
- **Surface used:** the static field **`TYPE_INT_RGB` only**. Never constructed
  (`createObject` without `.init()`), no instance methods, no pixel access. The
  value is passed straight into PDFBox's `writeImage()` as an int.

  ```cfml
  var bufferedImage = createObject("java","java.awt.image.BufferedImage");
  ...
  imageWriter.writeImage( document, "jpg", "", "1", "1", returnFilePrefix,
                          bufferedImage.TYPE_INT_RGB, arguments.width );
  ```
- **Shim cost:** **none.** `TYPE_INT_RGB` is the constant `1`. Preside can use
  `JavaCast("int",1)` and drop the `java.awt` dependency entirely.
- **But:** does **not** unblock the feature on its own — `org.apache.pdfbox.*` on
  the next two lines (entry 2) is the real blocker.
- **Boot path:** no. Upload-time only.
- **Cross-ref:** `docs/known-issues.md:529` already records that
  `imageGetBufferedImage` throws 🛑 because it would return a `java.awt.BufferedImage`.
  This is the *only* other `java.awt` reference in core Preside — there is no
  broader AWT surface to support.

## 2. `org.apache.pdfbox.util.PDFImageWriter`, `org.apache.pdfbox.pdmodel.PDDocument` 🛑

- **Call sites:** `NativeImageService.cfc:259-263`
- **Feature:** PDF thumbnails / previews for uploaded PDF assets.
- **Surface used:**
  - `PDDocument.load( filePath )` → doc; `doc.close()`
  - `PDFImageWriter.writeImage( doc, format:string, password:string, startPage:string, endPage:string, filePrefix:string, imageType:int, width:int )` — writes `{prefix}1.jpg` to the temp dir as a side effect.
- **Notes:** `PDFImageWriter` is **PDFBox 1.x** API (removed in 2.x, replaced by
  `PDFRenderer`). Whatever supplies this must match the 1.x signature, or Preside
  needs updating. Downstream the CFC calls `cfimage action="resize"` (Tier-1,
  already implemented per `known-issues.md:499`) then `JavaImageMetaReader::readMeta`
  (commons-imaging, ✅ shimmed) — so **PDFBox is the only gap in this chain**.
- **Fallback if absent:** PDF assets get no preview image. Non-fatal.
- **Suggested shape:** a native PDF-raster shim exposing just `writeImage`'s
  first-page-to-JPG behaviour would cover Preside's entire usage.

## 3. `org.apache.batik.*` — SVG → PNG 🛑

- **Call sites:** `system/services/assetManager/SvgToPngService.cfc:22-26`
  (`image.PNGTranscoder`, `TranscoderInput`, `TranscoderOutput`)
- **Feature:** rasterising uploaded SVG assets (`@feature assetManager`).
- **Surface used:**
  - `PNGTranscoder().init()`; static hint keys `KEY_WIDTH`, `KEY_HEIGHT`
  - `t.addTranscodingHint( key, JavaCast("float", n) )`
  - `TranscoderInput.init( svgUriString )` / `TranscoderOutput.init( outputStream )`
  - `t.transcode( input, output )`
- **Also needs:** `java.io.File.toURL().toString()` (entry 8) and
  `java.io.FileOutputStream` (entry 7).
- **Fallback if absent:** the CFC `$raiseError()`s then **rethrows** — so this one
  surfaces as a hard error to the user, unlike most others here.
- **Suggested shape:** a `resvg`/`usvg`-backed native path would satisfy the whole
  surface; only width/height hints are used, no other transcoding options.

## 4. `com.opencsv.CSVWriter` 🛑

- **Call site:** `system/services/dataExport/CsvWriter.cfc:28` (`@feature dataExport`)
- **Surface used:** `.init( writer, JavaCast("char", delimiter) )`, then
  `writeNext( array )`, `flush()`, `close()`. That is the complete surface — four
  methods, wrapped 1:1 by the CFC.
- **Also needs:** `java.io.FileWriter` (entry 7).
- **Jar:** hardcoded path `/preside/system/services/dataExport/lib/opencsv-3.8.jar`.
- **Impact:** all CSV data exports and scheduled exports.
- **Suggested shape:** trivially implementable natively — RFC-4180 quoting with a
  configurable delimiter. Arguably Preside should do this in pure CFML.

## 5. `javax.mail.Session` 🛑

- **Call site:** `system/services/email/EmailService.cfc:145`
- **Feature:** **"test connection settings"** in the admin email config UI only —
  `validateConnectionSettings()`. Actual mail sending goes through `cfmail`, not this.
- **Surface used:** `Session.getInstance( props, null )` → `getTransport("smtp")` →
  `transport.connect( host, port, user, pass )` / `close()`. Also catches the typed
  exception `javax.mail.AuthenticationFailedException`.
- **Also needs:** `java.util.Properties` (entry 9) — `.init()`, `.put(k,v)` for
  `mail.smtp.starttls.enable` and `mail.smtp.auth`.
- **Fallback if absent:** admins can't test SMTP credentials; sending itself is
  unaffected. **Low priority** — narrow blast radius.

## 6. `net.glxn.qrgen.*` 🛑

- **Call sites:** `system/services/qrcodes/QrCodeGenerator.cfc:18` (`javase.QRCode`),
  `:43` (`core.image.ImageType`)
- **Surface used:** fluent chain
  `QRCode.from(text).to(imageType).withSize(w,h).stream().toByteArray()`, plus the
  static constants `ImageType.JPG` / `.PNG` / `.GIF`.
- **Jars:** `system/services/qrcodes/lib/` (recursive `DirectoryList`).
- **⚠️ Unrelated bug spotted while surveying:** `QrCodeGenerator.cfc:25-27` writes to
  a hardcoded path `/resources/qrcodes/helloWorld.jpg` whenever `imageType == "jpg"`.
  Looks like leftover debug code in core Preside — worth raising upstream regardless
  of the port.

## 7. `java.io.FileOutputStream`, `java.io.FileWriter`, `java.io.ByteArrayOutputStream` 🛑

**Not shimmed — these throw.** `docs/java-shims.md:163` explicitly lists
`java.io.InputStream`/`OutputStream`/generic `Reader`/`Writer` among the
deliberately-unsupported classes. (`java.io.File`, `FileInputStream` and
`InputStreamReader` *are* shimmed — easy to over-generalise from.)

| Class | Call sites | Surface used |
|---|---|---|
| `FileOutputStream` | `SvgToPngService.cfc:25`, `ChunkedUploadService.cfc:62` | `.init(pathString)`, `write(binary)`, `flush()`, `close()` |
| `FileWriter` | `CsvWriter.cfc:27` | `.init(path)` — then handed to OpenCSV, never touched directly |
| `ByteArrayOutputStream` | `PlantUmlDiagramService.cfc:16`, `GoogleAuthenticator.cfc:277` | `.init()`, `write()`, `toByteArray()`, `close()` |

**`ChunkedUploadService` is the one that matters** — chunked/resumable admin file
uploads stream every chunk through `FileOutputStream.write( FileReadBinary(chunk) )`
in a loop. Without it, large-file upload is dead.

## 8. `java.io.File` — `.toURL()` 🏗

- **Call site:** `SvgToPngService.cfc:23` — `File.init(path).toURL().toString()`
- `java.io.File` **is** shimmed, but `toURL()` (deprecated in the JDK in favour of
  `toURI().toURL()`) may not be. Per the silent-`null` caveat, this would surface as
  a confusing downstream failure in Batik rather than a clear error. Worth an
  explicit check.

## 9. `java.util.Properties`, `java.util.StringTokenizer`, `java.security.SecureRandom` 🛑

Used, absent from the shim list → **throw at construction**
(`java.security.MessageDigest` *is* shimmed; `SecureRandom` is not):

| Class | Call site | Surface used |
|---|---|---|
| `java.util.Properties` | `EmailService.cfc:140` | `.init()`, `.put(k,v)` |
| `java.util.StringTokenizer` | `EmailStyleInliner.cfc:132` | `.init(str, delims)`, `countTokens()`, `nextToken()` |
| `java.security.SecureRandom` | `GoogleAuthenticator.cfc:127` | `.init()`, `nextBytes(byteArray)` |

`StringTokenizer` sits in the email CSS-rule parser — reachable on **every styled
email send**, so higher-traffic than its obscurity suggests. `SecureRandom` mutates
a `byte[]` **in place** via `nextBytes()`; a shim must preserve that aliasing or 2FA
key generation silently produces zero-filled salts.

## 10. `javax.crypto.*` — 2FA / TOTP 🛑

- **File:** `system/services/authentication/GoogleAuthenticator.cfc`
- **Explicitly not shimmed.** `docs/java-shims.md:163` names `javax.crypto.*` in the
  unsupported list, with the note *"use CFML's built-in `hash()`, `encrypt()`,
  `hmac()` instead"* — so `createObject` throws and 2FA fails loudly at first use.
- **`hmac()` is the intended route.** `getOneTimeToken()` is doing HMAC-SHA1 by hand;
  CFML's built-in `hmac( msg, key, "HMACSHA1" )` covers it without any shim. The
  PBKDF2 path (`SecretKeyFactory` + `PBEKeySpec`) has no direct BIF equivalent and
  would need either a shim or a Preside-side change.
- Members used, for whoever implements either route:

| Class | Line | Surface used |
|---|---|---|
| `javax.crypto.spec.SecretKeySpec` | 84 | `.init( byte[], "HmacSHA1" )`, `getAlgorithm()` |
| `javax.crypto.Mac` | 85 | `getInstance(alg)`, `init(keySpec)`, `doFinal(byte[])` → `byte[]` |
| `javax.crypto.SecretKeyFactory` | 135 | `getInstance("PBKDF2WithHmacSHA1")`, `generateSecret(keySpec)` |
| `javax.crypto.spec.PBEKeySpec` | 136 | `.init( char[], salt, 128, 80 )` |
| `java.nio.ByteBuffer` | 86, 128, 163 | `allocate(n)`, `putLong(n)`, `array()` — also unshimmed 🛑 |

- **Impact:** admin two-factor auth — dead until addressed, but it fails at
  `createObject`, so it announces itself.
- **⚠️ Hazard for whoever writes the shim or the CFML rewrite:**
  `getOneTimeToken()` indexes the HMAC result as `h[20]` — a **1-based CFML index
  into a Java `byte[]`** — and relies on signed-byte semantics
  (`if (t < 0) t += 256`). An implementation returning unsigned bytes or a 0-based
  array breaks TOTP **silently, producing wrong codes rather than errors**. This is
  the one place in the survey where a *fix* is more dangerous than the *gap*.
  Needs a cross-engine test against known TOTP vectors.
- **Dead code:** `GoogleAuthenticator.cfc:316` references
  `createObject("java","java.lang.String")` inside a **comment** — the live line uses
  `charsetEncode()`. Ignore; it inflates naive greps.

## 11. `net.sourceforge.plantuml.*` 🛑

- **File:** `system/services/cfflow/util/PlantUmlDiagramService.cfc` (`@feature cfflow`)
- **Reached dynamically** via `_getPlantUmlObj()` — invisible to classname greps.
- **Surface used:** `SourceStringReader.init(uml)`,
  `reader.generateImage( outputStream, FileFormatOption )`,
  `FileFormat.SVG` (static), `FileFormatOption.init( format )`.
- **Impact:** workflow diagram rendering in the admin. Feature-flagged (`cfflow`),
  so likely off for most sites. **Lowest priority here** — a real PlantUML shim means
  reimplementing a diagram language; better treated as "feature unavailable".

---

# Part 2 — Already no-op'd (see `PRESIDE_BOOT_JAVA_NOOPS.md`)

Boot blockers, patched on `chrono-port`. Recorded here for completeness:

| Class | File | Status |
|---|---|---|
| `com.adobe.xmp.XMPMetaFactory` | `XmpMetaReader.cfc:64` | no-op'd, **guard added** ✅ |
| `org.jsoup.Jsoup` | `EmailLoggingService.cfc`, `EmailStyleInliner.cfc` | no-op'd, **guards added** ✅ |
| `org.owasp.validator.html.AntiSamy` + `.Policy` | `AntiSamyService.cfc:51,62` | no-op'd, guarded — ⚠️ **passes HTML through unsanitized** |
| `com.cronutils.*` | `CronUtil.cfc` | **replaced** by the pure-CFML `chrono` library |
| `java.util.concurrent.*` (executors) | `AbstractHeartBeat.cfc` | `start()` returns unconditionally |

> **Correction to `PRESIDE_BOOT_JAVA_NOOPS.md` §1:** that entry states "`readMeta()`
> already guards the parse behind `xmp.len() < source.len() && IsXml(xmp)`". That
> guard tests whether *XMP was found in the file*, **not** whether the factory
> exists — so a null factory still reached `_getMetaFactory().parseFromString()` and
> threw on any image carrying XMP. Same omission applied to both jsoup consumers.
> Real `IsSimpleValue()` guards have since been added to all three, each returning an
> empty result instead. **AntiSamy was the only one originally guarded correctly.**

---

# Part 3 — Full inventory

Every literal `CreateObject("java", …)` in `system/` outside externals, with shim
status per `docs/java-shims.md`. Counts are call sites, not distinct files.

## Covered ✅

| Class | Uses | Notable call sites |
|---|---|---|
| `java.lang.System` | 11 | `identityHashCode` (cachebox stores, `PresideObjectService:65`), `getenv()` (`EnvironmentVariablesReader:54`), `getProperty("file.separator")` (`ExtensionManagerService:213`), `currentTimeMillis()` |
| `java.lang.StringBuffer` | 6 | `FormsService`, `FormBuilderService` — render buffers |
| `java.util.concurrent.ConcurrentHashMap` | 4 | cachebox stores, `MetadataIndexer` |
| `java.util.Collections` | 4 | cachebox stores; `reverse()` in `WebsiteBenefitsManager:159` |
| `java.net.InetAddress` | 3 | `getLocalHost()` — `TaskManagerService:976`, `ScheduledExportService:349`, `errorReport.cfm:16` |
| `java.io.File` | 3 | `JavaImageMetaReader:19`, `AntiSamyService:78`, `SvgToPngService:23` (⚠️ `.toURL()`, entry 8) |
| `java.time.Instant` | 2 | `now().toEpochMilli()` — session management |
| `java.lang.Thread` | 2 | `ThreadUtil:10`; `currentThread().getThreadGroup().getName()` in `AdHocTaskManagerService:704` |
| `java.lang.ref.WeakReference` | 2 | workflow spec libraries |
| `java.lang.ref.SoftReference` / `ReferenceQueue` | 2 | `ConcurrentSoftReferenceStore` |
| `java.util.regex.Pattern` / `Matcher` | 2 | `DynamicFindAndReplaceService:65`, `EmailTemplateService:576` — ⚠️ `Matcher.replaceAll`/`replaceFirst`/`split` are listed as *known gaps* (`java-shims.md:160`); verify which members these two use |
| `java.lang.StringBuilder` | 1 | `Renderer.cfc:276` |
| `java.util.UUID` | 1 | `InterceptorState.cfc:16` |
| `java.util.PropertyResourceBundle` | 1 | `ResourceBundleService:223` (with `FileInputStream` + `InputStreamReader`) |
| `org.apache.commons.imaging.Imaging` | 1 | `JavaImageMetaReader:20` — `getImageInfo(file)` |
| `org.mindrot.jbcrypt.BCrypt` | 1 | `BCryptService.cfc:6` |

## Gaps — see Part 1

**🛑 Not shimmed, throws at construction — 26 call sites:**
`java.awt.image.BufferedImage` · `org.apache.pdfbox.*` (2) · `org.apache.batik.*` (3) ·
`net.glxn.qrgen.*` (2) · `net.sourceforge.plantuml.*` (dynamic ×3) · `com.opencsv.CSVWriter` ·
`javax.mail.Session` · `javax.crypto.*` (4) · `java.nio.ByteBuffer` (3) ·
`java.io.FileOutputStream` (2) · `java.io.FileWriter` · `java.io.ByteArrayOutputStream` (2) ·
`java.util.Properties` · `java.util.StringTokenizer` · `java.security.SecureRandom`

**🏗 Shimmed class, unverified member — 1 call site:**
`java.io.File.toURL()` (`SvgToPngService:23`) — the only silent-`null` exposure left.

## Tally

| Status | Call sites |
|---|---:|
| ✅ Covered | 47 |
| 🛑 Throws (unshimmed class) | 26 |
| 🏗 Silent-`null` risk (unshimmed member) | 1 |
| 🔇 No-op'd on `chrono-port` | 5 |
| ☠️ Dead/commented code | 1 |
| **Total literal call sites** | **80** |

Plus 5 dynamic sites invisible to the grep (3 PlantUML, 2 jsoup). `CronUtil`'s former
~6 Java call sites are absent because `chrono` replaced them outright.

**The 🛑/🏗 split is the useful one:** 26 of 27 gaps fail loudly at `createObject`,
so they're discoverable by exercising the feature. Only `File.toURL()` can fail
quietly. The dangerous silent failures are *not* in this list — they're the no-ops in
Part 2 (AntiSamy passing HTML through, heartbeats not running) and the TOTP
byte-semantics hazard in entry 10, which only bites once someone implements a fix.

## Suggested order of work

1. **`ChunkedUploadService` / `FileOutputStream`** (entry 7) — breaks large-file
   admin upload; core CMS workflow, no fallback.
2. **`StringTokenizer`** (entry 9) — on the hot path for every styled email.
3. **2FA** (entry 10) — but via CFML's built-in `hmac()` rather than a
   `javax.crypto` shim, per the engine docs' own recommendation. Mind the
   byte-semantics hazard.
4. **`com.opencsv`** (entry 4) — four methods; whole feature (data export) for cheap.
5. **`BufferedImage`** (entry 1) — free, but pointless until PDFBox lands.
6. Batik / PDFBox / qrgen — self-contained media features, degrade to "no preview".
7. `javax.mail` / PlantUML — narrowest impact; fine left unavailable.
