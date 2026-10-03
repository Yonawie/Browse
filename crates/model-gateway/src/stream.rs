//! Streaming types shared by providers, the router and the gateway.

use std::pin::Pin;

use futures_util::Stream;
use serde_json::Value;

use crate::{ModelError, ModelResponse};

/// Parsed server-sent-event payloads (`data:` lines) as produced by a transport.
pub type JsonStream = Pin<Box<dyn Stream<Item = Result<Value, ModelError>> + Send>>;

/// A chat stream yields text deltas and terminates with exactly one `Done`
/// carrying the assembled response (content, tool calls, usage).
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Delta(String),
    Done(ModelResponse),
}

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ModelError>> + Send>>;

/// Wrap a complete response as a two-event stream (used when the provider or
/// the cache already has the whole answer).
pub fn single(resp: ModelResponse) -> ChatStream {
    let mut events = vec![];
    if !resp.content.is_empty() {
        events.push(Ok(StreamEvent::Delta(resp.content.clone())));
    }
    events.push(Ok(StreamEvent::Done(resp)));
    Box::pin(futures_util::stream::iter(events))
}

/// Drain a stream, returning the final response (and the concatenated deltas
/// if the provider never sent `Done`, which is a protocol violation).
pub async fn collect(mut stream: ChatStream) -> Result<ModelResponse, ModelError> {
    use futures_util::StreamExt;
    let mut text = String::new();
    while let Some(ev) = stream.next().await {
        match ev? {
            StreamEvent::Delta(d) => text.push_str(&d),
            StreamEvent::Done(r) => return Ok(r),
        }
    }
    Err(ModelError::Protocol(format!("stream ended without Done (received {} chars)", text.len())))
}
