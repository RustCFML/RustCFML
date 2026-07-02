//! Native CPU/wall-clock sampling profiler — Phase 6 of the observability plan.
//!
//! Unlike the in-engine CFML-frame profiler (Phase 2, `profileNow()`), this
//! profiles the **Rust** side: bytecode dispatch, BIF internals, allocator
//! pressure — the hot spots the CFML-frame sampler can't see. It wraps
//! [`pprof`](https://docs.rs/pprof) (pprof-rs): a `SIGPROF` timer samples the
//! native stack at ~100 Hz with a malloc-free signal handler, and on completion
//! we emit both an interactive **flamegraph SVG** and a **pprof protobuf**
//! (loadable in `go tool pprof` / Pyroscope / speedscope).
//!
//! Enabled by the `obs-pprof` Cargo feature and armed at runtime with the
//! `--profile` flag on a one-shot CLI run. Unix-only (SIGPROF); the `pprof` dep
//! is declared under the unix target table, so this module is `unix`-gated too.

#![cfg(all(feature = "obs-pprof", unix))]

use std::fs::File;
use std::io::Write;

/// A live profiling session. Hold it for the duration of the work to profile;
/// call [`Session::finish`] to write the reports.
pub struct Session {
    guard: pprof::ProfilerGuard<'static>,
    flamegraph_path: String,
    pprof_path: String,
}

/// Start sampling the current process at ~100 Hz. Returns `None` (with a warning)
/// if the profiler can't be armed. `out_prefix` names the output files
/// (`<prefix>.svg` + `<prefix>.pb`).
pub fn start(out_prefix: &str) -> Option<Session> {
    match pprof::ProfilerGuardBuilder::default()
        .frequency(100)
        // Skip noise from the runtime/allocator internals.
        .blocklist(&["libc", "libgcc", "pthread", "vdso", "libdyld"])
        .build()
    {
        Ok(guard) => {
            eprintln!("profiler: sampling at 100Hz → {out_prefix}.svg + {out_prefix}.pb");
            Some(Session {
                guard,
                flamegraph_path: format!("{out_prefix}.svg"),
                pprof_path: format!("{out_prefix}.pb"),
            })
        }
        Err(e) => {
            eprintln!("profiler: failed to start ({e}); continuing without profiling");
            None
        }
    }
}

impl Session {
    /// Stop sampling and write the flamegraph SVG + pprof protobuf.
    pub fn finish(self) {
        let report = match self.guard.report().build() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("profiler: could not build report: {e}");
                return;
            }
        };

        match File::create(&self.flamegraph_path) {
            Ok(f) => {
                if let Err(e) = report.flamegraph(f) {
                    eprintln!("profiler: flamegraph write failed: {e}");
                } else {
                    eprintln!("profiler: wrote {}", self.flamegraph_path);
                }
            }
            Err(e) => eprintln!("profiler: cannot create {}: {e}", self.flamegraph_path),
        }

        match report.pprof() {
            Ok(profile) => {
                // `protobuf-codec` uses rust-protobuf, whose Message trait
                // serialises via `write_to_bytes()`.
                use pprof::protos::Message;
                match profile.write_to_bytes() {
                    Ok(buf) => {
                        if let Err(e) =
                            File::create(&self.pprof_path).and_then(|mut f| f.write_all(&buf))
                        {
                            eprintln!("profiler: pprof write failed: {e}");
                        } else {
                            eprintln!("profiler: wrote {}", self.pprof_path);
                        }
                    }
                    Err(e) => eprintln!("profiler: pprof encode failed: {e}"),
                }
            }
            Err(e) => eprintln!("profiler: pprof build failed: {e}"),
        }
    }
}
