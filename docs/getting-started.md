# Getting Started

[← Back to README](../README.md)

The fastest way to start is with a prebuilt binary — no toolchain required. Building from source is only needed if you want to contribute or build for a platform we don't ship.

## Prebuilt binaries

Download a binary for your platform from the [latest release](https://github.com/RustCFML/RustCFML/releases/latest):

- `rustcfml-linux-x86_64`
- `rustcfml-linux-aarch64`
- `rustcfml-macos-aarch64`

Make it executable and put it on your `PATH`:

```bash
chmod +x rustcfml-macos-aarch64
sudo mv rustcfml-macos-aarch64 /usr/local/bin/rustcfml
rustcfml --version
```

### Licensing

RustCFML is MIT-licensed. Each release also carries `LICENSE` and
`THIRD-PARTY.txt` (the attribution notices for the ~530 Rust crates linked into
the binary) as downloadable assets. Both are embedded in the binary too, so you
can read them from a binary you have already installed:

```bash
rustcfml --licenses
```

## Running CFML

```bash
rustcfml myapp.cfm                       # Run a .cfm template
rustcfml -c 'writeOutput("Hello!")'      # Inline code
rustcfml -r                              # Interactive REPL
rustcfml --serve ./mywebroot --port 8500 # Web server (see Web Server docs)
rustcfml --licenses                      # MIT licence + third-party notices
```

See **[Web Server](web-server.md)** for serve mode and **[Deployment](deployment.md)** for packaging apps as standalone binaries.

## Shell scripts (shebang support)

RustCFML scripts can be executed directly as shell scripts using a shebang line. The file extension does not matter.

```bash
#!/usr/bin/env rustcfml
writeOutput("Hello from a shell script!" & chr(10));
var x = 2 + 2;
writeOutput("2 + 2 = " & x & chr(10));
```

```bash
chmod +x myscript.cfm
./myscript.cfm
```

## Building from source

You need Rust stable (>= 1.75.0) — install via [rustup.rs](https://rustup.rs/).

```bash
git clone https://github.com/RustCFML/RustCFML.git
cd RustCFML
cargo build --release            # binary at target/release/rustcfml
cargo install --path crates/cli  # optional: install on your PATH
```

Run the test suite to verify your build (see **[Testing](testing.md)**):

```bash
cargo run -- tests/runner.cfm    # CFML test suite
cargo test                       # Rust unit tests
```

### Feature flags

Database drivers are feature-gated in `cfml-stdlib` so unused subsystems compile out. SQLite is on by default; enable others as needed:

```bash
cargo build --release --features "mysql_db,postgres_db,mssql_db"
```

See **[Database](database.md)** for datasource configuration and **[WebAssembly](wasm.md)** for the `wasm32` target.
