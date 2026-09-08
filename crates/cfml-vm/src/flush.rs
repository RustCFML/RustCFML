//! `<cfflush>` — pushing buffered output to the client mid-request.
//!
//! Normally a request's output accumulates in `output_buffer` and the embedder
//! turns the whole thing into one HTTP response *after* execution finishes, so
//! the status line and headers can still be decided at the end. `<cfflush>`
//! breaks that deal: it sends what has been produced so far, which **commits**
//! the response — the status and headers travel with that first chunk and can
//! never be revised.
//!
//! Reference behaviour was read out of both engines rather than guessed:
//!
//! * **Lucee** (`lucee.runtime.tag.Flush`) calls `getRootOut().flush()`, where
//!   `getRootOut()` is `bodyContentStack.getBase()` — the *base* writer. So a
//!   flush inside `<cfsavecontent>` pushes the page content written before the
//!   capture began and leaves the capture itself intact.
//! * **BoxLang** (`…components.system.Flush`) calls `context.flushBuffer(true)`.
//!   The `force` flag also walks to the request context, but its
//!   `BaseBoxContext.flushBuffer` drags *every* registered buffer out with it,
//!   so a flush inside a capture leaks the captured text to the client.
//!
//! We follow Lucee (and ACF): flush targets the **root** buffer and a
//! `<cfsavecontent>` / `<cfsilent>` / custom-tag body capture is never
//! disturbed. See `docs/known-issues.md` for the recorded divergence.

use cfml_common::session_cookie::SessionCookiePolicy;

/// Everything the embedder needs to write the HTTP status line and headers,
/// snapshotted at the instant the first `<cfflush>` commits the response.
///
/// After this is handed over, later `cfheader`/`cfcookie` writes are dropped on
/// the floor (servlet-spec parity: `setHeader` on a committed response is a
/// no-op) and `cflocation` / `cfcontent reset` / `cfhtmlhead` raise an error.
#[derive(Debug, Clone, Default)]
pub struct FlushCommit {
    pub status: Option<(u16, String)>,
    pub content_type: Option<String>,
    pub headers: Vec<(String, String)>,
    pub session_id: Option<String>,
    pub session_record_created: bool,
    pub session_cookie_policy: SessionCookiePolicy,
}

/// Carries flushed output out of a still-running request.
///
/// Installed by the server; in CLI mode there is no sink and a flush writes
/// straight to stdout. Called on the request's own (blocking) thread, so an
/// implementation may block.
pub trait FlushSink: Send {
    /// Emit one chunk. `commit` is `Some` exactly once — on the first flush,
    /// carrying the response metadata that must precede the first body byte.
    ///
    /// Returns `false` when the peer has gone away; the VM records that and
    /// stops trying to flush, but does **not** abort the request (matching
    /// Lucee, whose `flush()` failure is only fatal when `throwonerror`).
    fn flush(&mut self, commit: Option<FlushCommit>, chunk: String) -> bool;
}

use crate::CfmlVirtualMachine;
use cfml_common::vm::{CfmlError, CfmlErrorType};

impl CfmlVirtualMachine {
    /// The buffer a flush drains: the **base** of the capture stack, never the
    /// innermost one. Mirrors Lucee's `getRootOut()`; see the module docs for
    /// why BoxLang differs.
    fn root_output_buffer(&mut self) -> &mut String {
        if self.saved_output_buffers.is_empty() {
            &mut self.output_buffer
        } else {
            &mut self.saved_output_buffers[0]
        }
    }

    /// Snapshot the response metadata that has to precede the first body byte.
    fn flush_commit(&mut self) -> FlushCommit {
        FlushCommit {
            status: self.response_status.clone(),
            content_type: self.response_content_type.clone(),
            headers: self.response_headers.clone(),
            session_id: self.session_id.clone(),
            session_record_created: self.session_record_created,
            session_cookie_policy: self.session_cookie_policy.clone(),
        }
    }

    /// Push the root buffer to the client. Returns `Err` only for a genuine
    /// transport failure, which `<cfflush throwonerror="false">` swallows.
    ///
    /// A flush from inside a `cfthread` body is a no-op: the thread writes into
    /// its own captured buffer and has no client of its own (Lucee gives the
    /// thread a separate PageContext, so its `getRootOut()` is not the
    /// request's either).
    pub fn cfflush(&mut self) -> Result<(), CfmlError> {
        if self.in_thread_body > 0 {
            return Ok(());
        }
        // The peer already hung up on an earlier flush — stay quiet rather than
        // erroring on every subsequent one.
        if self.output_sink_gone {
            return Ok(());
        }

        if self.flush_sink.is_some() {
            let chunk = std::mem::take(self.root_output_buffer());
            let first = !self.response_flushed;
            // The response is committed even by an empty first flush: that is
            // how `<cfflush>` before any output ships the headers early.
            self.response_flushed = true;
            let commit = if first { Some(self.flush_commit()) } else { None };

            let sink = self.flush_sink.as_mut().expect("checked above");
            if !sink.flush(commit, chunk) {
                self.output_sink_gone = true;
                return Err(CfmlError::new(
                    "The connection to the client was lost while flushing output.".to_string(),
                    CfmlErrorType::Application,
                ));
            }
            return Ok(());
        }

        // No sink. What that means depends on who is running us.
        if self.http_request_data.is_some() {
            // A web request whose embedder cannot stream this response (the
            // onMissingTemplate and error-template paths build one buffered
            // body). Degrade to "keep buffering": the page still renders
            // correctly and in order, it simply arrives in one piece. Crucially
            // this does NOT commit — the headers are still open, so nothing
            // that follows starts failing — and it must never print to the
            // SERVER's stdout, which is not the client.
            return Ok(());
        }

        // CLI / embedded: the root output IS stdout, so a flush genuinely is
        // "print what we have now" rather than at the end of the run. Nothing
        // is committed because there are no response headers to freeze — which
        // is also why `<cflocation>` and friends keep working afterwards.
        let chunk = std::mem::take(self.root_output_buffer());
        if !chunk.is_empty() {
            use std::io::Write;
            print!("{}", chunk);
            let _ = std::io::stdout().flush();
        }
        Ok(())
    }

    /// `<cfflush interval="N">`: flush now if the root buffer already exceeds N
    /// bytes, and keep doing so as output accumulates. Lucee's
    /// `setBufferConfig(N, autoFlush=true)`, which calls `_check()` immediately.
    pub fn cfflush_set_interval(&mut self, interval: usize) -> Result<(), CfmlError> {
        self.flush_interval = Some(interval);
        self.check_auto_flush()
    }

    /// The `_check()` half of Lucee's writer: with an interval configured,
    /// flush as soon as the root buffer passes it. Called from the output
    /// chokepoints; a no-op (one `Option` test) when no interval is set.
    #[inline]
    pub(crate) fn check_auto_flush(&mut self) -> Result<(), CfmlError> {
        let Some(interval) = self.flush_interval else {
            return Ok(());
        };
        if self.in_thread_body > 0 {
            return Ok(());
        }
        if self.root_output_buffer().len() > interval {
            self.cfflush()?;
        }
        Ok(())
    }

    /// Guard for the operations that cannot work once the response is on the
    /// wire.
    ///
    /// Messages and types are Lucee's, verified against a running Lucee 7.1
    /// rather than assumed — the servlet spec would suggest `cfheader` is
    /// silently ignored once the response is committed, but Lucee's
    /// `Header.doStartTag` throws on `isCommitted()`. The full matrix:
    ///
    /// | after `<cfflush>`        | Lucee                                                        |
    /// |--------------------------|--------------------------------------------------------------|
    /// | `<cfflush>` again        | fine                                                         |
    /// | `<cfabort>`              | fine                                                         |
    /// | `<cfcookie>`             | fine (silently ineffective)                                  |
    /// | `<cfheader>`             | throws `template`: can't assign value to header…             |
    /// | `<cfcontent>` (any)      | throws `application`: Content was already flushed            |
    /// | `<cflocation>`           | throws: Response buffer is already flushed                   |
    /// | `<cfhtmlhead>`/`body`    | throws: Page is already flushed                              |
    pub(crate) fn error_if_flushed(
        &self,
        message: &str,
        error_type: CfmlErrorType,
    ) -> Result<(), CfmlError> {
        if self.response_flushed {
            return Err(CfmlError::new(message.to_string(), error_type));
        }
        Ok(())
    }
}
