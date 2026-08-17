# Java Shim Support

[← Back to README](../README.md)

RustCFML has **no JVM under the hood**, so `createObject("java", …)` and `<cfobject type="java">` are served by hand-written **shims** — pure-Rust emulations of a curated set of Java classes that real-world CFML frameworks (ColdBox, Preside, Taffy, Wheels, etc.) reach for. The goal is to run those libraries, not to reimplement the JDK.

```cfml
// Constructor + chaining
sb = createObject("java", "java.lang.StringBuilder").init("Hello");
sb.append(", World").append("!");
writeOutput(sb.toString());   // Hello, World!

// Static method
now = createObject("java", "java.lang.System").currentTimeMillis();

// UUID
id = createObject("java", "java.util.UUID").randomUUID().toString();

// java.time (JSR-310) date math
dt = createObject("java", "java.time.LocalDateTime").now().plusDays(7);
writeOutput(dt.toString());
```

> **Expect differences.** These are emulations, not the real classes. Anything outside the lists below is **not** shimmed. Class names are matched case-insensitively, so `java.lang.System` and `java.lang.system` both work.

## Important caveats

- **Unsupported classes throw.** `createObject("java", "java.util.HashMap")` raises `createObject: Java class [java.util.HashMap] is not supported.` rather than silently returning null. (This changed from the old "returns null" behaviour — a missing class now fails loudly at construction.)
- **Unsupported *methods* on a supported class usually return `null`** (no error), so subsequent calls can fail in confusing ways. See **Known gaps** below. A few classes are stricter and *throw* on an unknown method — the deferred class-loader tower, `java.net.URL` network methods, and the third-party library shims (BCrypt, SnakeYAML, JSON-schema, commons-imaging).
- **Many values are hardcoded, not measured.** `Runtime.freeMemory()`/`totalMemory()`/`maxMemory()`, `Thread.getPriority()` (5), `UUID.getVersion()` (4), `Process.isAlive()` (false) and similar return fixed values — enough for framework boot, not for real introspection. (`TimeZone` offsets were in this list; as of v0.551.0 they are real, chrono-tz-backed values including DST.)
- **`java.security.MessageDigest` hashes correctly.** `getInstance(alg)` + `update(bytes)` + `digest()` produces a real digest — `SHA-256` of `"abc"` is `BA7816BF…15AD`, as it should be. An algorithm the shim does not implement raises `java.security.NoSuchAlgorithmException` rather than quietly substituting MD5 (v0.551.0). Supported: `MD5`, `SHA`/`SHA-1`, `SHA-256`, `SHA-384`, `SHA-512`. `MessageDigest.isEqual(a, b)` does a correct constant-shape byte-array comparison. *(This entry previously said `digest()` "does NOT actually hash" and steered callers away — that was wrong.)*
- **`java.security.Signature` really signs and verifies.** `getInstance("SHA256withRSA")` + `initVerify`/`initSign` + `update` + `verify`/`sign` performs genuine RSASSA-PKCS1-v1_5 — the surface vendored JWT libraries (jwt-cfml and friends) use for RS256, so an unmodified library verifies real Auth0 id tokens, including the JWKS `n`/`e` path through `BigInteger` + `RSAPublicKeySpec`. Signing is deterministic and byte-identical to OpenSSL. Scope is **RSA verify/sign only**: EC/ECDSA, `CertificateFactory` and `KeyPairGenerator` are not shimmed, and a non-RSA algorithm throws rather than being verified under the wrong scheme.
- **Regex uses the Rust [`regex`](https://docs.rs/regex) crate**, whose syntax is a close superset of common Java `Pattern` usage but is **not identical** (no backreferences/lookaround). `\uXXXX` escapes are translated. Invalid patterns throw a clear `java.util.regex.Pattern: invalid pattern …` error.
- **The Java `Thread` shim is a stub.** `Thread.sleep()` is a no-op and it offers no concurrency. This is **separate** from CFML's `<cfthread>`, which *does* run on real OS threads — see **[Threading](threads.md)**.
- **`java.util.concurrent` executors run tasks, but simply.** `submit`/`execute`/`invokeAll`/`invokeAny` spawn work through RustCFML's async kernel, but the pool is not a real thread pool: statistics (`getActiveCount`, `getPoolSize`, …) are 0, `awaitTermination` always returns true, and **periodic scheduling (`scheduleAtFixedRate`/`scheduleWithFixedDelay`) fires only once**, not repeatedly.
- **No true immutability/thread-safety.** `Collections.unmodifiableList()` / `synchronizedMap()` are identity operations (matching Lucee's practical behaviour), and the "concurrent" collections are single-threaded emulations.
- **`java.lang.ref.*` never clears.** `SoftReference`/`WeakReference` hold their referent strongly forever (no JVM GC), and a `ReferenceQueue` is permanently empty.
- **`System.getProperty("java.version")` returns `"rustcfml"`** — a deliberate tell that there is no JVM.

## Shimmed classes

Unless noted as *(indirect)*, each class is constructible via `createObject("java", …)` and/or `.init()`. Static methods are called on the class object (e.g. `createObject("java","java.util.UUID").randomUUID()`).

### java.lang

| Class (and aliases) | Supported methods |
|---|---|
| `java.lang.System` | static `currentTimeMillis()`, `nanoTime()`, `getProperty(key[, default])`, `setProperty(k, v)`, `clearProperty(k)`, `getenv([name])`, `identityHashCode(o)`, and `System.out`/`System.err` fields |
| `java.lang.System.out` / `.err` *(indirect, via `System.out`/`System.err`)* | `println`, `print`, `write`, `append`, `printf`, `format`; `flush`/`close`/`checkError` are no-ops |
| `java.lang.StringBuilder` / `java.lang.StringBuffer` | `init([s])`, `append(v)` (chainable), `toString()`, `length()`, `clear()`, `insert(i,v)`, `delete(from,to)`, `deleteCharAt(i)`, `setLength(n)`, `replace(from,to,s)`, `reverse()` — all mutate in place and return the builder, so a builder passed into a function is mutated for the caller |
| `java.lang.Thread` | static `currentThread()`; `getName()`, `getThreadGroup()`, `getPriority()` (→ 5), `isDaemon()` (→ false), `sleep()` (no-op) |
| `java.lang.ThreadGroup` *(indirect, via `Thread.getThreadGroup()`)* | `getName()` |
| `java.lang.Runtime` | static `getRuntime()`; `availableProcessors()`, `freeMemory()`/`totalMemory()`/`maxMemory()` (hardcoded), `gc()`/`runFinalization()` (no-op) |
| `java.lang.ProcessBuilder` | `init(command…)`, `command(…)`, `start()` (runs the process synchronously → `Process` shim) |
| `java.lang.Process` *(indirect, via `ProcessBuilder.start()`)* | `waitFor()`, `exitValue()`, `isAlive()` (→ false), `destroy()`/`destroyForcibly()` (no-op) |
| `java.lang.Class` *(also produced by any value's `getClass()`)* | static `forName(name)`; `getName()`, `getCanonicalName()`, `getTypeName()`, `getSimpleName()`, `toString()` |
| `java.lang.reflect.Array` | static `newInstance(type, len)`, `get(arr, i)`, `set(arr, i, v)`, `getLength(arr)` (0-based indices) |
| `java.lang.ref.SoftReference` / `java.lang.ref.WeakReference` | `init(referent[, queue])`, `get()`, `clear()`, `isEnqueued()` (→ false), `enqueue()` (→ false), `hashCode()` — *never cleared* |
| `java.lang.ref.ReferenceQueue` | `init()`, `poll()`/`remove()` — *always empty* |
| `java.lang.ClassLoader` / `java.net.URLClassLoader` / `coldfusion.runtime.java.JavaProxy` | "deferred" tower: `loadClass`/`forName` return a `Class` shim, `getSystemClassLoader`/`getParent`/`getContextClassLoader` return self, `addURL`/`setContextClassLoader` no-op, `getURLs`/`getName`/`toString`. **Any other method throws** ("no JVM") |

### java.util

| Class (and aliases) | Supported methods |
|---|---|
| `java.util.UUID` | static `randomUUID()`; `toString()`, `getVersion()` (→ 4), `getVariant()` (→ 2) |
| `java.util.Date` *(also from `Calendar.getTime()`)* | `init([millis])`, `getTime()`, `setTime(millis)`, `before(d)`, `after(d)`, `equals(d)`, `compareTo(d)` |
| `java.util.GregorianCalendar` / `java.util.Calendar` | `init([y,m,d[,h,mi,s]])` (month 0-based), `getTime()` (→ `Date`), `getTimeInMillis()`, `setTime(date)`, `setTimeInMillis(ms)`, `get(field)`, `set(field,v)` / `set(y,m,d,…)`, `add(field,n)`, `roll(field,n)`; the `YEAR`/`MONTH`/`DATE`/… field constants are exposed as properties. `add` carries into larger fields, `roll` wraps within the field |
| `java.util.TreeMap` | `init([struct])`, `put(k, v)`, `get(k)`, `keySet()`/`keys()` (**sorted**), `size()`, `containsKey(k)`, `isEmpty()`, `remove(k)` |
| `java.util.LinkedHashMap` | `init([struct])`, `put(k, v)`, `get(k)`, `keySet()`/`keys()` (insertion order), `size()`, `containsKey(k)`, `isEmpty()`, `remove(k)` |
| `java.util.Optional` | static `empty()`, `of(v)`, `ofNullable(v)`; `isPresent()`, `isEmpty()`, `get()` (throws if empty), `ifPresent(fn)`, `map(fn)`, `filter(fn)`, `orElse(v)`, `orElseGet(supplier)` (lazy — only called when empty), `orElseThrow([supplier])`, `equals()`, `hashCode()`, `toString()`. Note a real JVM rejects a CFML closure for `orElseGet`/`orElseThrow`; the shim accepts one |
| `java.util.Collections` | `list(e)`, `emptyList()`/`emptySet()`/`emptyMap()`, `sort(list)` (natural ordering — numeric for all-numeric lists, lexicographic otherwise), `reverse(list)`, and identity `unmodifiable*`/`synchronized*` wrappers |
| `java.util.Iterator` *(indirect, via `array.iterator()` / queue `iterator()`)* | `hasNext()`, `next()` (throws past end) |
| `java.util.Enumeration` *(indirect, via `ResourceBundle.getKeys()`)* | `hasMoreElements()`/`hasNext()`, `nextElement()`/`next()` |
| `java.util.Map.Entry` *(indirect, via `ConcurrentHashMap.entrySet()`)* | `getKey()`, `getValue()`, `toString()` |
| `java.util.Locale` | static `getDefault()`, `getAvailableLocales()`, `getISOLanguages()`, `getISOCountries()`; `getLanguage()`, `getCountry()`, `getVariant()`, `getDisplayLanguage()`, `getDisplayCountry()`, `getDisplayName()`, `getISO3Language()`, `getISO3Country()`, `toString()` |
| `java.util.TimeZone` | static `getDefault()`, `getTimeZone(id)`, `getAvailableIDs()`; `getID()`, `getDisplayName()`, `getRawOffset()`/`getDSTSavings()`/`getOffset(millis)`, `useDaylightTime()`/`inDaylightTime()` — real chrono-tz values, DST included (`America/New_York` is -18000000 in January and -14400000 in July) |
| `java.util.PropertyResourceBundle` | `init(reader)` (parses a `.properties` file), `getKeys()`/`keySet()` (→ `Enumeration`), `getString(k)`/`getObject(k)`/`handleGetObject(k)`, `containsKey(k)` |

### java.util.concurrent

| Class (and aliases) | Supported methods |
|---|---|
| `java.util.concurrent.Executors` | static `newFixedThreadPool()`, `newCachedThreadPool()`, `newSingleThreadExecutor()`, `newWorkStealingPool()`, `newScheduledThreadPool()`, `newSingleThreadScheduledExecutor()`, `defaultThreadFactory()` |
| `ThreadPoolExecutor` / `ScheduledThreadPoolExecutor` / `AbstractExecutorService` | `submit`, `execute`, `invokeAll`, `invokeAny`, `schedule`/`scheduleAtFixedRate`/`scheduleWithFixedDelay` (**fire once**); `shutdown`, `shutdownNow`, `isShutdown`, `isTerminated`, `awaitTermination` (→ true), `getActiveCount`/`getPoolSize`/`getTaskCount`/… (→ 0), `getQueue` (empty) |
| `java.util.concurrent.ExecutorCompletionService` | `init(executor)`, `submit(task)`, `poll()`/`take()` (FIFO completed futures) |
| `java.util.concurrent.TimeUnit` | constant fields `NANOSECONDS`…`DAYS`; `toString()`/`name()` |
| `java.util.concurrent.ThreadFactory` | `newThread(runnable)` |
| `java.util.concurrent.ConcurrentHashMap` | `init()`, `put(k, v)`, `putIfAbsent(k, v)`, `get(k)`, `getOrDefault(k, d)`, `replace(k, v)`, `remove(k)`, `containsKey(k)`, `keys()`/`keySet()`/`values()`, `entrySet()` (→ `Map.Entry` array), `size()`, `isEmpty()`, `clear()`. `compute`/`computeIfAbsent`/`computeIfPresent`/`merge` **throw** — they need a remapping function the shim cannot invoke |
| `java.util.concurrent.ConcurrentLinkedQueue` *(aliases `LinkedQueue`, `LinkedBlockingQueue`, `ArrayBlockingQueue`)* | `init()`, `add(v)`/`offer(v)`, `poll()`, `peek()`, `remove()`, `contains(v)`, `drainTo(sink[, max])`, `iterator()` (→ `Iterator`), `size()`, `isEmpty()`, `clear()`. `take()` **throws** — it would have to block |

### java.io / java.nio.file / java.net

| Class (and aliases) | Supported methods |
|---|---|
| `java.io.File` | `init(path)`, `toString()`, `getAbsolutePath()`, `getCanonicalPath()`, `isAbsolute()`, `exists()`, `isFile()`, `isDirectory()`, `getName()`, `getParent()`, `lastModified()`, `length()`, `toPath()`, `toURL()`/`toURI()`, `mkdir()`/`mkdirs()`, `delete()`, `createNewFile()`, plus `separator`/`pathSeparator` fields |
| `java.io.FileInputStream` | `init(path\|File)`, `close()` (no-op) |
| `java.io.InputStreamReader` | `init(FileInputStream[, charset])`, `close()` (no-op) |
| `java.nio.file.Paths` / `java.nio.file.Path` *(also via `File.toPath()`)* | static `get(s)`; `getParent()`, `isAbsolute()`, `toString()`, `toAbsolutePath()` |
| `java.nio.file.Files` | static `exists(p)`, `isDirectory(p)`, `isSymbolicLink(p)`, `delete(p)`, `write(p, content)`, `copy(src,dst)`, `move(src,dst)`, `createDirectory(p)`/`createDirectories(p)`, `readAllBytes(p)` (→ byte array). Paths may be a `Path` shim or a plain string. I/O failure raises `java.io.IOException` |
| `java.net.InetAddress` | static `getLocalHost()`, `getByName(host)`; `getHostName()`, `getHostAddress()`, `getCanonicalHostName()`, `isLoopbackAddress()`, `toString()`. `getByName` resolves through the system resolver and raises `java.net.UnknownHostException` when it cannot; IP literals and `localhost` short-circuit without a lookup |
| `java.net.URL` | `init(spec)` (or protocol/host/port/file), `getProtocol()`, `getHost()`, `getPort()`, `getDefaultPort()`, `getPath()`, `getQuery()`, `getRef()`, `getFile()`, `getAuthority()`, `getUserInfo()`, `toString()`/`toExternalForm()`, `equals()`. **`openConnection`/`openStream`/`getContent`/`getInputStream` throw** — use `<cfhttp>` |

### java.security / java.util.regex / java.text

| Class (and aliases) | Supported methods |
|---|---|
| `java.security.MessageDigest` | static `getInstance(algorithm)`, `isEqual(a, b)`; `update(data)`, `digest()`, `reset()`. Real hashing — `MD5`, `SHA`/`SHA-1`, `SHA-256`, `SHA-384`, `SHA-512`; an unknown algorithm throws `NoSuchAlgorithmException` |
| `java.security.Signature` | static `getInstance(algorithm)`; `initVerify(publicKey)`, `initSign(privateKey)`, `update(data)` (accumulates across calls), `verify(sigBytes)`, `sign()`, `getAlgorithm()`. Real RSASSA-PKCS1-v1_5 — `SHA1withRSA`, `SHA256withRSA`, `SHA384withRSA`, `SHA512withRSA`. **RSA only**: a non-RSA cipher (`SHA256withECDSA`) throws `NoSuchAlgorithmException` rather than verifying under the wrong scheme. `verify()` returns `false` for a bad signature — it does not throw |
| `java.security.KeyFactory` | static `getInstance("RSA")`; `generatePublic(spec)` (from `X509EncodedKeySpec` or `RSAPublicKeySpec`), `generatePrivate(spec)` (from `PKCS8EncodedKeySpec`), `getAlgorithm()`. A malformed key raises `InvalidKeySpecException` at generate time |
| `java.security.spec.X509EncodedKeySpec` / `.PKCS8EncodedKeySpec` | `init(derBytes)` — the DER a PEM body base64-decodes to; `getEncoded()`, `getFormat()` |
| `java.security.spec.RSAPublicKeySpec` | `init(modulus, exponent)` taking two `BigInteger`s — the JWKS `n`/`e` path; `getModulus()`, `getPublicExponent()` |
| `java.security.PublicKey` / `PrivateKey` *(indirect, via `KeyFactory`)* | `getAlgorithm()`, `getFormat()`, `getEncoded()` |
| `java.math.BigInteger` | `init(signum, magnitudeBytes)` — **signum-magnitude, not two's-complement**, which is what a JWKS modulus with its high bit set needs — or `init("decimal")`; `bitLength()`, `signum()`, `toString()`, `toByteArray()` |
| `java.util.regex.Pattern` | static/instance `compile(regex)`, `pattern()`/`toString()`, `matcher(input)` |
| `java.util.regex.Matcher` *(indirect, via `Pattern.matcher()`)* | `find()`, `matches()`, `lookingAt()`, `group([n])`, `groupCount()`, `start([n])`, `end([n])` |
| `java.text.DateFormat` / `java.text.SimpleDateFormat` | static `getDateInstance()`, `getTimeInstance()`, `getDateTimeInstance()`, `getInstance()`; `format(date)`, `setTimeZone(tz)`; `setLenient`/`setCalendar`/`applyPattern` (no-op). **Only `en`/`en_US`/`en_GB` locales** — others throw |
| `java.text.DateFormatSymbols` | `getMonths()`, `getShortMonths()`, `getWeekdays()`, `getShortWeekdays()`, `getAmPmStrings()`, `getEras()` (English only) |
| `java.text.DecimalFormatSymbols` | `getPercent()`, `getMinusSign()`, `getCurrencySymbol()`, `getDecimalSeparator()`, `getGroupingSeparator()`, `getExponentSeparator()`, `getZeroDigit()`, `getInfinity()`, `getNaN()`, … (US locale) |

### java.time (JSR-310)

A chrono-backed shim covering enough of JSR-310 for ColdBox's async/scheduler date library. Arithmetic is real; a few exotic operations (`with(adjuster)`, `truncatedTo`, custom `format(pattern)`) are benign identity/best-effort. Instants are UTC epoch-millis internally.

| Class | Supported methods |
|---|---|
| `java.time.LocalDateTime` / `LocalDate` / `ZonedDateTime` | static `now()`, `parse(s)`, `ofEpochSecond(n)`; `plus*/minus*` (Days/Hours/Minutes/Seconds/Weeks/Months/Years), `plus`/`minus`, `with*` (Hour/Minute/Second/Nano/Year/Month/DayOfMonth), `isBefore`/`isAfter`/`isEqual`, `atZone`/`atStartOfDay`, `toInstant`, `toLocalDate(Time)`, `getYear`/`getMonthValue`/`getDayOfMonth`/`getHour`/`getMinute`/`getSecond`/`getDayOfWeek`, `toEpochSecond`/`toEpochMilli`, `format`/`toString`; `truncatedTo`/`with` are identity |
| `java.time.Instant` | static `now()`, `ofEpochMilli(n)`, `ofEpochSecond(n)`; `toEpochMilli`, `getEpochSecond`, `atZone`, `plusMillis`/`plusSeconds`, `isBefore`/`isAfter`, `toString` |
| `java.time.Duration` | static `ofDays`/`ofHours`/`ofMinutes`/`ofSeconds`/`ofMillis`/`ofNanos`/`of`/`between`; `toMillis`/`getSeconds`/`toMinutes`/`toHours`/`toDays`/`getNano`, `plus`/`minus`, `plus*`, `with*`, `isNegative`/`isZero`, `toString` |
| `java.time.Period` | static `ofDays`/`ofWeeks`/`ofMonths`/`ofYears`/`of`; `getDays`/`getMonths`/`getYears` |
| `java.time.ZoneId` / `java.time.ZoneOffset` | static `of(id)`, `systemDefault()`; `getId`/`toString`/`getDisplayName`/`normalized` (`ZoneOffset.UTC` field) |
| `java.time.DayOfWeek` | constants `MONDAY`…`SUNDAY`; static `of(n)`, `getValue()` |
| `java.time.Month` | constants `JANUARY`…`DECEMBER`; `getValue()` |
| `java.time.temporal.ChronoUnit` | constants `NANOS`…`FOREVER`; `valueOf(name)` |
| `java.time.temporal.ChronoField` / `TemporalAdjusters` | construct with constant fields only — no method dispatch |

### Servlet bridge (`lucee.runtime.*`)

**Not created via `createObject`** — these are produced by `getPageContext()` and drive real request/response state in serve mode.

| Class | Notes |
|---|---|
| `lucee.runtime.PageContextImpl` | `getRequest()`/`getResponse()`, `getRequestTimeout()`/`setRequestTimeout()`, and response forwards (`getStatus`, `setStatus`, `setContentType`, `addHeader`, `sendRedirect`, …) |
| `HttpServletRequestWrap` | read-only accessors synthesized from CGI scope: `getRequestURL`, `getRequestURI`, `getQueryString`, `getMethod`, `getScheme`, `getServerName`, `getServerPort`, `getHeader`, `isSecure`, … |
| `HttpServletResponseDummy` | drives `response_status`/`response_headers`: `setStatus`, `getStatus`, `setHeader`, `addHeader`, `containsHeader`, `setContentType`, `sendRedirect`, … |

### Third-party library shims (non-`java.*`)

These route real work to native Rust builtins; an unknown method **throws** rather than returning null.

| Class | Backed by | Methods |
|---|---|---|
| `org.mindrot.jbcrypt.BCrypt` | `bcryptHash`/`bcryptVerify` | `gensalt()`, `hashpw()`, `checkpw()` |
| `org.yaml.snakeyaml.Yaml` | `yamlSerialize`/`yamlDeserialize` | `load()`/`loadAs()`, `dump()`/`dumpAsMap()` |
| `ca.vanmulligen.json.schema.Validator` | JSON-schema validator | `init(schema[, baseUri])`, `isValid(json)` |
| `org.apache.commons.imaging.Imaging` | `imageInfo`/`imageRead` | `getImageInfo()` (→ `ImageInfo`), `getBufferedImage()` |
| `org.apache.commons.imaging.ImageInfo` *(indirect)* | — | `getWidth`, `getHeight`, `getFormatName`, `getBitsPerPixel`, `getColorType`, … |

## Known gaps

Methods on otherwise-supported classes that are **not** implemented (they return `null`):

- **`java.io.File`** — `canRead()`/`canWrite()`/`canExecute()`, `getParentFile()`, `listFiles()`. (`renameTo()` is implemented as of v0.551.0.)
- **`java.lang.StringBuilder`** — `substring()`. (`insert()`, `delete()`, `deleteCharAt()`, `setLength()`, `replace()` and `reverse()` are implemented as of v0.551.0.)
- **`TreeMap` / `LinkedHashMap`** — `values()`, `entrySet()`, `clear()`.
- **`java.nio.file.Paths`** — `getFileName()`, `getNameCount()`, `normalize()`, `relativize()`.
- **`java.util.regex`** — `Matcher.replaceAll()`/`replaceFirst()`/`split()`, `Pattern.matches()` (static), `Pattern.quote()`.
- **`java.time`** — `with(adjuster)` and `truncatedTo(unit)` are identity (return the receiver unchanged); `format(pattern)` ignores custom patterns and emits ISO-8601.

Whole classes commonly requested but **not** shimmed include `java.util.HashMap`/`ArrayList`, `java.io.InputStream`/`OutputStream`/generic `Reader`/`Writer`, `java.sql.*`, and `javax.crypto.*` (use CFML's built-in `hash()`, `encrypt()`, `hmac()` instead). Requesting one of these from `createObject` now **throws** rather than returning null.

If you hit a missing class or method that a framework needs, please [open an Issue](https://github.com/RustCFML/RustCFML/issues) with the call and the framework context — the shim set grows from real-world demand.

## Tests

The shims are exercised by the CFML suite under [`tests/java_shims/`](../tests/java_shims/) — e.g. `test_all.cfm`, `test_comprehensive.cfm`, `test_security.cfm`, `test_concurrent_map.cfm`, `test_stringbuilder.cfm`, `test_file.cfm`, `test_java_url.cfm`, `test_optional.cfm`, `test_property_bundle.cfm`, `test_classloader_shims.cfm`, `test_commons_imaging.cfm`, `test_system_properties.cfm` — which is also run against Lucee to confirm the emulated behaviour matches the reference engine. See **[Testing](testing.md)**.
