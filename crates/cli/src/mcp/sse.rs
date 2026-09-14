//! Server-Sent Events for the MCP HTTP transport.
//!
//! The engine had no SSE before this, so the wire format is encoded here by
//! hand and the body is built with the same `mpsc → Body::from_stream` pattern
//! `<cfflush>` already uses (`build_streaming_response` in `lib.rs`). That
//! keeps the dependency set unchanged and reuses a path known to frame
//! correctly under hyper.
//!
//! Three framing rules earn their comments, because breaking any of them
//! produces a stream that looks fine in a test and fails against a real client:
//! every line of the payload needs its own `data:` prefix, the event ends with
//! a blank line, and no `Content-Length` may be set.

use std::sync::Arc;

use axum::response::Response;
use cfml_vm::mcp::McpSink;

/// Outbound queue depth per stream. On overflow the stream is closed rather
/// than the server blocking — a client that cannot drain its notifications
/// must not be able to stall a tool that is running on a blocking worker.
const OUTBOUND_QUEUE: usize = 256;

/// How many events a stream retains for `Last-Event-ID` resumption. Bounded so
/// a client that disconnects and never returns cannot pin memory.
pub(crate) const REPLAY_CAP: usize = 256;

/// Keep-alive comment interval. Without it an idle standalone stream is killed
/// by intermediate proxies, and the client sees an unexplained disconnect.
const KEEPALIVE: std::time::Duration = std::time::Duration::from_secs(15);

/// What the registry pushes at a stream.
enum StreamCmd {
    Event { id: String, data: String },
    Close,
}

/// The [`McpSink`] handed to the registry for one SSE stream.
#[derive(Debug)]
pub(crate) struct SseSink {
    tx: tokio::sync::mpsc::Sender<StreamCmd>,
}

impl McpSink for SseSink {
    fn send(&self, event_id: &str, payload: &str) -> bool {
        // Non-blocking: the registry is called from synchronous VM code.
        self.tx
            .try_send(StreamCmd::Event { id: event_id.to_string(), data: payload.to_string() })
            .is_ok()
    }

    fn close(&self) {
        let _ = self.tx.try_send(StreamCmd::Close);
    }
}

/// Encode one SSE event.
///
/// The per-line `data:` prefix is mandatory, not defensive: a payload
/// containing a newline would otherwise terminate the event early and put the
/// remainder on the wire as a malformed frame. JSON-RPC messages are serialized
/// compactly so this is normally a single line, but CFML strings carry
/// newlines and `\r` of their own.
fn encode(id: &str, data: &str) -> bytes::Bytes {
    let mut s = String::with_capacity(data.len() + 64);
    s.push_str("id: ");
    s.push_str(id);
    s.push('\n');
    for line in data.split('\n') {
        s.push_str("data: ");
        s.push_str(line.trim_end_matches('\r'));
        s.push('\n');
    }
    s.push('\n');
    bytes::Bytes::from(s)
}

/// A live SSE stream: the sink to register, and the response to return.
pub(crate) struct Stream {
    pub(crate) sink: Arc<SseSink>,
    rx: tokio::sync::mpsc::Receiver<StreamCmd>,
}

impl Stream {
    pub(crate) fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(OUTBOUND_QUEUE);
        Stream { sink: Arc::new(SseSink { tx }), rx }
    }

    /// Build the streaming response, optionally preceded by replayed events.
    ///
    /// `session` is echoed back in `Mcp-Session-Id` when the stream belongs to
    /// a freshly-minted session.
    pub(crate) fn into_response(
        self,
        session: Option<&str>,
        replay: Vec<(String, String)>,
    ) -> Response {
        let mut builder = axum::response::Response::builder()
            .status(200)
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache, no-store")
            // Reverse proxies buffer by default, which turns a progress stream
            // into one silent pause followed by everything at once.
            .header("X-Accel-Buffering", "no");
        if let Some(id) = session {
            builder = builder.header("Mcp-Session-Id", id);
        }
        // Deliberately no Content-Length: hyper then frames the body chunked,
        // exactly as the <cfflush> path relies on.

        let mut rx = self.rx;
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<bytes::Bytes>(8);
        tokio::spawn(async move {
            for (id, data) in replay {
                if out_tx.send(encode(&id, &data)).await.is_err() {
                    return;
                }
            }
            loop {
                let next = tokio::time::timeout(KEEPALIVE, rx.recv()).await;
                let cmd = match next {
                    // Idle: an SSE comment keeps proxies from dropping us.
                    Err(_elapsed) => {
                        if out_tx.send(bytes::Bytes::from_static(b":ping\n\n")).await.is_err() {
                            return;
                        }
                        continue;
                    }
                    Ok(None) => return, // every sender dropped
                    Ok(Some(cmd)) => cmd,
                };
                match cmd {
                    StreamCmd::Event { id, data } => {
                        if out_tx.send(encode(&id, &data)).await.is_err() {
                            return; // client went away
                        }
                    }
                    StreamCmd::Close => return,
                }
            }
        });

        let stream = futures_util::stream::unfold(out_rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (Ok::<bytes::Bytes, std::io::Error>(chunk), rx))
        });
        builder.body(axum::body::Body::from_stream(stream)).expect("valid SSE response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(b: bytes::Bytes) -> String {
        String::from_utf8(b.to_vec()).expect("utf8")
    }

    #[test]
    fn a_single_line_payload_encodes_as_one_data_field() {
        assert_eq!(text(encode("0-1", r#"{"a":1}"#)), "id: 0-1\ndata: {\"a\":1}\n\n");
    }

    #[test]
    fn every_line_gets_its_own_data_prefix() {
        // A bare newline inside the payload would otherwise end the event and
        // put the rest on the wire as a malformed frame.
        let out = text(encode("0-2", "line one\nline two"));
        assert_eq!(out, "id: 0-2\ndata: line one\ndata: line two\n\n");
    }

    #[test]
    fn carriage_returns_are_stripped_from_line_ends() {
        // A stray \r before the newline is invisible in a diff and breaks
        // strict client parsers.
        let out = text(encode("0-3", "a\r\nb"));
        assert_eq!(out, "id: 0-3\ndata: a\ndata: b\n\n");
    }

    #[test]
    fn an_event_always_ends_with_a_blank_line() {
        assert!(text(encode("0-4", "x")).ends_with("\n\n"), "a client waits forever otherwise");
    }

    #[tokio::test]
    async fn the_sink_reports_a_full_queue_rather_than_blocking() {
        let stream = Stream::new();
        // Fill the bounded queue; nothing is draining it.
        for _ in 0..OUTBOUND_QUEUE {
            assert!(stream.sink.send("0-0", "x"));
        }
        assert!(
            !stream.sink.send("0-0", "x"),
            "overflow must report failure so the registry closes the stream"
        );
    }
}
