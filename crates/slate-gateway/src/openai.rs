//! Anthropic's Messages API on one side, OpenAI's chat completions on the other.
//!
//! The agent CLI speaks the Messages API. A request is rewritten into a chat
//! completions request, and the answer (whole, or a stream of chunks) into what the
//! Messages API would have returned. Only what an agent turn needs is carried: text,
//! images, tool calls and their results, the stop reason and the token counts.
//! Reasoning that a provider streams is dropped; so are Anthropic's server tools.

use serde_json::{json, Map, Value};

/// What a provider turned out not to accept, learnt from its error messages so the
/// next request is right the first time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Quirks {
    /// Send `max_completion_tokens` instead of `max_tokens` (OpenAI's newer models).
    pub max_completion_tokens: bool,
    /// The largest output limit the provider takes, when smaller than the CLI asks for.
    pub max_tokens_cap: Option<u64>,
    /// The model takes only its default temperature.
    pub no_temperature: bool,
    /// The provider does not know `stream_options`.
    pub no_stream_options: bool,
}

impl Quirks {
    /// Read a 400's message; true when it named something to change before a retry.
    pub fn learn(&mut self, message: &str, asked_max: Option<u64>) -> bool {
        let m = message.to_ascii_lowercase();
        let before = self.clone();
        if m.contains("max_completion_tokens") && !self.max_completion_tokens {
            self.max_completion_tokens = true;
        } else if m.contains("stream_options") {
            self.no_stream_options = true;
        } else if m.contains("temperature") {
            self.no_temperature = true;
        } else if m.contains("max_tokens") || m.contains("max tokens") {
            // "Range of max_tokens should be [1, 8192]": the largest number below what was asked.
            let asked = asked_max.unwrap_or(u64::MAX);
            let cap = numbers(&m)
                .into_iter()
                .filter(|n| *n > 1 && *n < asked)
                .max();
            if let Some(cap) = cap {
                self.max_tokens_cap = Some(self.max_tokens_cap.map_or(cap, |c| c.min(cap)));
            }
        }
        *self != before
    }
}

fn numbers(text: &str) -> Vec<u64> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse().ok())
        .collect()
}

/// The text of a content value: a string, or the text blocks of a list.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                Some("text") => b.get("text").and_then(Value::as_str).map(str::to_string),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// An Anthropic image block as a chat completions content part.
fn image_part(block: &Value) -> Option<Value> {
    let source = block.get("source")?;
    let url = match source.get("type").and_then(Value::as_str)? {
        "base64" => format!(
            "data:{};base64,{}",
            source
                .get("media_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png"),
            source.get("data").and_then(Value::as_str)?
        ),
        "url" => source.get("url").and_then(Value::as_str)?.to_string(),
        _ => return None,
    };
    Some(json!({"type": "image_url", "image_url": {"url": url}}))
}

/// Text and image parts as a chat message's content: a plain string when there is
/// only text (every provider takes that), a list of parts otherwise.
fn user_content(texts: Vec<String>, images: Vec<Value>) -> Value {
    if images.is_empty() {
        return Value::String(texts.join("\n"));
    }
    let mut parts: Vec<Value> = texts
        .into_iter()
        .filter(|t| !t.is_empty())
        .map(|t| json!({"type": "text", "text": t}))
        .collect();
    parts.extend(images);
    Value::Array(parts)
}

/// One Messages API message as chat messages, appended to `out`.
fn push_message(message: &Value, out: &mut Vec<Value>) {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user");
    let content = message.get("content").cloned().unwrap_or(Value::Null);
    if role == "system" {
        // A note from the harness in the middle of the conversation. Many chat
        // models take a system message only at the start, so it goes as the
        // person's turn, marked the way the CLI marks its own notes.
        let text = text_of(&content);
        if !text.is_empty() {
            let text = if text.contains("<system-reminder>") {
                text
            } else {
                format!("<system-reminder>\n{text}\n</system-reminder>")
            };
            out.push(json!({"role": "user", "content": text}));
        }
        return;
    }
    let blocks = match content {
        Value::String(s) => {
            out.push(json!({"role": role, "content": s}));
            return;
        }
        Value::Array(blocks) => blocks,
        _ => return,
    };
    if role == "assistant" {
        let mut text = vec![];
        let mut calls = vec![];
        for b in &blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = b.get("text").and_then(Value::as_str) {
                        text.push(t.to_string());
                    }
                }
                Some("tool_use") => calls.push(json!({
                    "id": b.get("id").cloned().unwrap_or(Value::Null),
                    "type": "function",
                    "function": {
                        "name": b.get("name").cloned().unwrap_or(Value::Null),
                        "arguments": b.get("input").cloned().unwrap_or_else(|| json!({})).to_string(),
                    },
                })),
                _ => {} // thinking, server tool use: nothing a chat model can take back
            }
        }
        if text.is_empty() && calls.is_empty() {
            return;
        }
        let mut m = Map::new();
        m.insert("role".into(), json!("assistant"));
        m.insert(
            "content".into(),
            if text.is_empty() {
                Value::Null
            } else {
                Value::String(text.join("\n"))
            },
        );
        if !calls.is_empty() {
            m.insert("tool_calls".into(), Value::Array(calls));
        }
        out.push(Value::Object(m));
        return;
    }
    // A user message: tool results first (a chat model expects them right after the
    // call), then what the person or the harness added.
    let mut texts = vec![];
    let mut images = vec![];
    for b in &blocks {
        match b.get("type").and_then(Value::as_str) {
            Some("tool_result") => {
                let inner = b.get("content").cloned().unwrap_or(Value::Null);
                let mut text = text_of(&inner);
                let pictures: Vec<Value> = inner
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|x| x.get("type").and_then(Value::as_str) == Some("image"))
                            .filter_map(image_part)
                            .collect()
                    })
                    .unwrap_or_default();
                if b.get("is_error").and_then(Value::as_bool) == Some(true)
                    && !text.starts_with("Error")
                {
                    text = format!("Error: {text}");
                }
                if text.is_empty() {
                    text = if pictures.is_empty() {
                        "(no output)".into()
                    } else {
                        "(see the image that follows)".into()
                    };
                }
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": b.get("tool_use_id").cloned().unwrap_or(Value::Null),
                    "content": text,
                }));
                // A tool message carries text only: its pictures go to the user turn.
                images.extend(pictures);
            }
            Some("text") => {
                if let Some(t) = b.get("text").and_then(Value::as_str) {
                    texts.push(t.to_string());
                }
            }
            Some("image") => images.extend(image_part(b)),
            _ => {}
        }
    }
    if !texts.is_empty() || !images.is_empty() {
        out.push(json!({"role": "user", "content": user_content(texts, images)}));
    }
}

/// Consecutive user messages as one: the CLI sends its notes as turns of their own,
/// and some chat models insist that user and assistant turns alternate.
fn merge_user_turns(messages: Vec<Value>) -> Vec<Value> {
    fn parts(content: &Value) -> Vec<Value> {
        match content {
            Value::String(s) => vec![json!({"type": "text", "text": s})],
            Value::Array(a) => a.clone(),
            _ => vec![],
        }
    }
    let mut out: Vec<Value> = vec![];
    for m in messages {
        let user = m.get("role").and_then(Value::as_str) == Some("user");
        match out.last_mut() {
            Some(last) if user && last.get("role").and_then(Value::as_str) == Some("user") => {
                let merged = match (&last["content"], &m["content"]) {
                    (Value::String(a), Value::String(b)) => Value::String(format!("{a}\n\n{b}")),
                    (a, b) => Value::Array(parts(a).into_iter().chain(parts(b)).collect()),
                };
                last["content"] = merged;
            }
            _ => out.push(m),
        }
    }
    out
}

/// A Messages API request as a chat completions request for `model`.
pub fn chat_request(request: &Value, model: &str, quirks: &Quirks) -> Value {
    let mut messages = vec![];
    let system = request.get("system").map(text_of).unwrap_or_default();
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system}));
    }
    for m in request
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        push_message(m, &mut messages);
    }
    let messages = merge_user_turns(messages);
    let mut out = Map::new();
    out.insert("model".into(), json!(model));
    out.insert("messages".into(), Value::Array(messages));
    let stream = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if stream {
        out.insert("stream".into(), json!(true));
        if !quirks.no_stream_options {
            out.insert("stream_options".into(), json!({"include_usage": true}));
        }
    }
    if let Some(max) = request.get("max_tokens").and_then(Value::as_u64) {
        let max = quirks.max_tokens_cap.map_or(max, |cap| max.min(cap));
        let key = if quirks.max_completion_tokens {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        out.insert(key.into(), json!(max));
    }
    if !quirks.no_temperature {
        if let Some(t) = request.get("temperature").filter(|t| t.is_number()) {
            out.insert("temperature".into(), t.clone());
        }
    }
    if let Some(p) = request.get("top_p").filter(|p| p.is_number()) {
        out.insert("top_p".into(), p.clone());
    }
    if let Some(stop) = request
        .get("stop_sequences")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty())
    {
        out.insert("stop".into(), Value::Array(stop.clone()));
    }
    // Function tools only: Anthropic's own (web search, computer use) run on its
    // servers and have no counterpart here.
    let tools: Vec<Value> = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("input_schema").is_some() && matches!(t.get("type").and_then(Value::as_str), None | Some("custom")))
        .map(|t| {
            json!({"type": "function", "function": {
                "name": t.get("name").cloned().unwrap_or(Value::Null),
                "description": t.get("description").cloned().unwrap_or_else(|| json!("")),
                "parameters": t.get("input_schema").cloned().unwrap_or_else(|| json!({"type": "object"})),
            }})
        })
        .collect();
    if !tools.is_empty() {
        out.insert("tools".into(), Value::Array(tools));
        let choice = request.get("tool_choice");
        match choice.and_then(|c| c.get("type")).and_then(Value::as_str) {
            Some("any") => {
                out.insert("tool_choice".into(), json!("required"));
            }
            Some("none") => {
                out.insert("tool_choice".into(), json!("none"));
            }
            Some("tool") => {
                let name = choice
                    .and_then(|c| c.get("name"))
                    .cloned()
                    .unwrap_or(Value::Null);
                out.insert(
                    "tool_choice".into(),
                    json!({"type": "function", "function": {"name": name}}),
                );
            }
            _ => {}
        }
    }
    Value::Object(out)
}

fn stop_reason(finish: Option<&str>, has_calls: bool) -> &'static str {
    match finish {
        _ if has_calls => "tool_use",
        Some("length") => "max_tokens",
        Some("tool_calls") | Some("function_call") => "tool_use",
        _ => "end_turn",
    }
}

/// Token counts in the Messages API's terms. A chat completions `prompt_tokens`
/// includes what was read from the cache; the Messages API counts that apart.
fn usage(chat: Option<&Value>) -> (u64, u64, u64) {
    let get = |v: Option<&Value>, k: &str| {
        v.and_then(|u| u.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let cached = chat
        .and_then(|u| u.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let prompt = get(chat, "prompt_tokens");
    (
        prompt.saturating_sub(cached),
        cached,
        get(chat, "completion_tokens"),
    )
}

fn usage_json(chat: Option<&Value>) -> Value {
    let (input, cached, output) = usage(chat);
    json!({"input_tokens": input, "output_tokens": output, "cache_read_input_tokens": cached, "cache_creation_input_tokens": 0})
}

/// A tool call's arguments as the object the Messages API wants.
fn arguments(text: &str) -> Value {
    serde_json::from_str::<Value>(text)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// A tool call's id for the CLI. The CLI tells calls apart by id and ignores one it
/// has seen in the conversation, and some servers number theirs from zero in every
/// answer ("call_0") or send none: those get an id of our own.
fn call_id(given: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if given.len() >= 10 {
        return given.to_string();
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!(
        "call_{:08x}{:04x}",
        nanos & 0xffff_ffff,
        NEXT.fetch_add(1, Ordering::Relaxed) & 0xffff
    )
}

fn message_id(chat_id: Option<&str>) -> String {
    match chat_id {
        Some(id) if !id.is_empty() => format!("msg_{}", id.trim_start_matches("chatcmpl-")),
        _ => "msg_gateway".into(),
    }
}

/// A whole chat completion as a Messages API message.
pub fn message(chat: &Value, model: &str) -> Value {
    let choice = chat.get("choices").and_then(|c| c.get(0));
    let msg = choice.and_then(|c| c.get("message"));
    let mut content = vec![];
    let text = msg
        .and_then(|m| m.get("content"))
        .map(text_of)
        .unwrap_or_default();
    if !text.is_empty() {
        content.push(json!({"type": "text", "text": text}));
    }
    let calls = msg
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array);
    for call in calls.into_iter().flatten() {
        let f = call.get("function");
        content.push(json!({
            "type": "tool_use",
            "id": call_id(call.get("id").and_then(Value::as_str).unwrap_or("")),
            "name": f.and_then(|f| f.get("name")).cloned().unwrap_or(Value::Null),
            "input": arguments(f.and_then(|f| f.get("arguments")).and_then(Value::as_str).unwrap_or("")),
        }));
    }
    let has_calls = calls.is_some_and(|c| !c.is_empty());
    json!({
        "id": message_id(chat.get("id").and_then(Value::as_str)),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": stop_reason(choice.and_then(|c| c.get("finish_reason")).and_then(Value::as_str), has_calls),
        "stop_sequence": null,
        "usage": usage_json(chat.get("usage")),
    })
}

/// One server-sent event of the Messages API: its name and its data.
pub type SseEvent = (&'static str, Value);

#[derive(Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
}

/// Turns the chunks of a streamed chat completion into the Messages API's events.
/// Text goes out as it arrives. Tool calls are held until the end: chat completions
/// may interleave several, the Messages API has them one block after another.
pub struct StreamTranslator {
    model: String,
    started: bool,
    finished: bool,
    text_open: bool,
    blocks: usize,
    calls: Vec<Call>,
    finish: Option<String>,
    usage: Option<Value>,
}

impl StreamTranslator {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            started: false,
            finished: false,
            text_open: false,
            blocks: 0,
            calls: vec![],
            finish: None,
            usage: None,
        }
    }

    fn start(&mut self, chat_id: Option<&str>, out: &mut Vec<SseEvent>) {
        if self.started {
            return;
        }
        self.started = true;
        out.push(("message_start", json!({"type": "message_start", "message": {
            "id": message_id(chat_id), "type": "message", "role": "assistant", "model": self.model,
            "content": [], "stop_reason": null, "stop_sequence": null, "usage": usage_json(None),
        }})));
        out.push(("ping", json!({"type": "ping"})));
    }

    /// The events one chunk gives rise to.
    pub fn push(&mut self, chunk: &Value) -> Vec<SseEvent> {
        let mut out = vec![];
        if self.finished {
            return out;
        }
        self.start(chunk.get("id").and_then(Value::as_str), &mut out);
        if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage = Some(u.clone());
        }
        let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
            return out;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
            {
                if !self.text_open {
                    self.text_open = true;
                    out.push((
                        "content_block_start",
                        json!({"type": "content_block_start", "index": self.blocks,
                        "content_block": {"type": "text", "text": ""}}),
                    ));
                }
                out.push((
                    "content_block_delta",
                    json!({"type": "content_block_delta", "index": self.blocks,
                    "delta": {"type": "text_delta", "text": text}}),
                ));
            }
            for call in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                while self.calls.len() <= index {
                    self.calls.push(Call::default());
                }
                let slot = &mut self.calls[index];
                if let Some(id) = call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|i| !i.is_empty())
                {
                    slot.id = id.to_string();
                }
                let f = call.get("function");
                if let Some(name) = f.and_then(|f| f.get("name")).and_then(Value::as_str) {
                    slot.name.push_str(name);
                }
                if let Some(a) = f.and_then(|f| f.get("arguments")).and_then(Value::as_str) {
                    slot.arguments.push_str(a);
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(reason.to_string());
        }
        out
    }

    /// The events that close the message, once the stream has ended.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = vec![];
        if self.finished {
            return out;
        }
        self.start(None, &mut out);
        self.finished = true;
        if self.text_open {
            out.push((
                "content_block_stop",
                json!({"type": "content_block_stop", "index": self.blocks}),
            ));
            self.blocks += 1;
        }
        let calls: Vec<Call> = std::mem::take(&mut self.calls)
            .into_iter()
            .filter(|c| !c.name.is_empty())
            .collect();
        for call in calls.iter() {
            let id = call_id(&call.id);
            out.push((
                "content_block_start",
                json!({"type": "content_block_start", "index": self.blocks,
                "content_block": {"type": "tool_use", "id": id, "name": call.name, "input": {}}}),
            ));
            out.push(("content_block_delta", json!({"type": "content_block_delta", "index": self.blocks,
                "delta": {"type": "input_json_delta", "partial_json": arguments(&call.arguments).to_string()}})));
            out.push((
                "content_block_stop",
                json!({"type": "content_block_stop", "index": self.blocks}),
            ));
            self.blocks += 1;
        }
        let (input, cached, output) = usage(self.usage.as_ref());
        out.push(("message_delta", json!({"type": "message_delta",
            "delta": {"stop_reason": stop_reason(self.finish.as_deref(), !calls.is_empty()), "stop_sequence": null},
            "usage": {"input_tokens": input, "output_tokens": output, "cache_read_input_tokens": cached, "cache_creation_input_tokens": 0}})));
        out.push(("message_stop", json!({"type": "message_stop"})));
        out
    }

    /// The events for a whole (unstreamed) chat completion, when a stream was asked for.
    pub fn whole(&mut self, chat: &Value) -> Vec<SseEvent> {
        let msg = chat
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"));
        let delta = json!({
            "content": msg.and_then(|m| m.get("content")).map(text_of).unwrap_or_default(),
            "tool_calls": msg.and_then(|m| m.get("tool_calls")).and_then(Value::as_array).map(|calls| calls.iter().enumerate()
                .map(|(i, c)| { let mut c = c.clone(); c["index"] = json!(i); c }).collect::<Vec<_>>()).unwrap_or_default(),
        });
        let finish = chat
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("finish_reason"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut out = self.push(&json!({"id": chat.get("id"), "usage": chat.get("usage"),
            "choices": [{"delta": delta, "finish_reason": finish}]}));
        out.extend(self.finish());
        out
    }
}

/// A rough count for `count_tokens`, which chat completions has no call for: the
/// request's text at three and a half characters a token.
pub fn estimate_tokens(request: &Value) -> u64 {
    fn chars(v: &Value) -> usize {
        match v {
            Value::String(s) => s.chars().count(),
            Value::Array(a) => a.iter().map(chars).sum(),
            Value::Object(o) => o
                .iter()
                .filter(|(k, _)| k.as_str() != "data")
                .map(|(_, v)| chars(v))
                .sum(),
            _ => 0,
        }
    }
    let n = ["system", "messages", "tools"]
        .iter()
        .filter_map(|k| request.get(*k))
        .map(chars)
        .sum::<usize>();
    (n as f64 / 3.5).ceil() as u64
}

/// The Messages API's error type for an HTTP status.
pub fn error_type(status: u16) -> &'static str {
    match status {
        400 | 422 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        529 => "overloaded_error",
        _ => "api_error",
    }
}

/// The message inside a provider's error body, whatever its shape.
pub fn error_message(body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let found = parsed.as_ref().and_then(|v| {
        let e = v.get("error").unwrap_or(v);
        e.get("message")
            .or_else(|| e.get("msg"))
            .or_else(|| v.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| e.as_str().map(str::to_string))
    });
    found.unwrap_or_else(|| {
        body.chars()
            .take(400)
            .collect::<String>()
            .trim()
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_with_tools_becomes_chat_messages() {
        let request = json!({
            "model": "claude-sonnet", "max_tokens": 32000, "stream": true, "temperature": 1,
            "system": [{"type": "text", "text": "You operate this machine.", "cache_control": {"type": "ephemeral"}}],
            "tools": [
                {"name": "Bash", "description": "Run a command", "input_schema": {"type": "object", "properties": {"command": {"type": "string"}}}},
                {"type": "web_search_20250305", "name": "web_search"}
            ],
            "messages": [
                {"role": "user", "content": "how much disk is free?"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "df", "signature": "x"},
                    {"type": "text", "text": "Let me look."},
                    {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "df -h /"}}]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": [{"type": "text", "text": "21G free"}]},
                    {"type": "text", "text": "<system-reminder>be brief</system-reminder>"}]}
            ]
        });
        let chat = chat_request(&request, "gpt-test", &Quirks::default());
        assert_eq!(chat["model"], "gpt-test");
        assert_eq!(chat["max_tokens"], 32000);
        assert_eq!(chat["stream_options"]["include_usage"], true);
        let m = chat["messages"].as_array().unwrap();
        assert_eq!(
            m[0],
            json!({"role": "system", "content": "You operate this machine."})
        );
        assert_eq!(
            m[1],
            json!({"role": "user", "content": "how much disk is free?"})
        );
        assert_eq!(m[2]["content"], "Let me look.");
        assert_eq!(
            m[2]["tool_calls"][0]["function"]["arguments"],
            "{\"command\":\"df -h /\"}"
        );
        assert_eq!(
            m[3],
            json!({"role": "tool", "tool_call_id": "toolu_1", "content": "21G free"})
        );
        assert_eq!(m.len(), 5);
        assert_eq!(
            m[4],
            json!({"role": "user", "content": "<system-reminder>be brief</system-reminder>"})
        );
        // Only the function tool; Anthropic's server tool has no counterpart.
        assert_eq!(chat["tools"].as_array().unwrap().len(), 1);
        assert_eq!(
            chat["tools"][0]["function"]["parameters"]["properties"]["command"]["type"],
            "string"
        );
    }

    #[test]
    fn harness_notes_join_the_person_s_turn() {
        let request = json!({"system": "S", "messages": [
            {"role": "user", "content": "<system-reminder>today is Thursday</system-reminder>"},
            {"role": "user", "content": [{"type": "text", "text": "what day is it?"}]},
            {"role": "system", "content": "The person switched windows."},
            {"role": "assistant", "content": [{"type": "text", "text": "Thursday."}]},
            {"role": "user", "content": "thanks"}]});
        let chat = chat_request(&request, "m", &Quirks::default());
        let m = chat["messages"].as_array().unwrap();
        let roles: Vec<&str> = m.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"]);
        assert_eq!(m[1]["content"], "<system-reminder>today is Thursday</system-reminder>\n\nwhat day is it?\n\n<system-reminder>\nThe person switched windows.\n</system-reminder>");
    }

    #[test]
    fn a_picture_in_a_tool_result_moves_to_the_user_turn() {
        let request = json!({"messages": [{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": [
                {"type": "text", "text": "the window"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}}]}]}]});
        let chat = chat_request(&request, "m", &Quirks::default());
        let m = chat["messages"].as_array().unwrap();
        assert_eq!(m[0]["role"], "tool");
        assert_eq!(m[0]["content"], "the window");
        assert_eq!(m[1]["role"], "user");
        assert_eq!(
            m[1]["content"][0]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
    }

    #[test]
    fn quirks_are_learnt_from_error_messages() {
        let mut q = Quirks::default();
        assert!(q.learn("Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.", Some(32000)));
        assert!(q.max_completion_tokens);
        assert!(q.learn(
            "Invalid max_tokens value, the valid range of max_tokens is [1, 8192]",
            Some(32000)
        ));
        assert_eq!(q.max_tokens_cap, Some(8192));
        assert!(q.learn(
            "Unsupported value: 'temperature' does not support 0.7 with this model.",
            None
        ));
        assert!(!q.learn("the model does not exist", None));
        let chat = chat_request(
            &json!({"max_tokens": 32000, "temperature": 0.7, "stream": true, "messages": []}),
            "m",
            &q,
        );
        assert_eq!(chat["max_completion_tokens"], 8192);
        assert!(chat.get("max_tokens").is_none() && chat.get("temperature").is_none());
    }

    fn names(events: &[SseEvent]) -> Vec<&'static str> {
        events.iter().map(|e| e.0).collect()
    }

    #[test]
    fn streamed_text_then_tool_calls() {
        let mut t = StreamTranslator::new("gpt-test");
        let mut ev = t.push(&json!({"id": "chatcmpl-9", "choices": [{"delta": {"role": "assistant", "content": ""}}]}));
        assert_eq!(names(&ev), ["message_start", "ping"]);
        ev = t.push(&json!({"choices": [{"delta": {"content": "Let me "}}]}));
        assert_eq!(names(&ev), ["content_block_start", "content_block_delta"]);
        ev = t.push(&json!({"choices": [{"delta": {"content": "look."}}]}));
        assert_eq!(ev[0].1["delta"]["text"], "look.");
        // Two calls, their fragments interleaved.
        t.push(&json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_a1b2c3d4e5", "function": {"name": "Bash", "arguments": "{\"comm"}}]}}]}));
        t.push(&json!({"choices": [{"delta": {"tool_calls": [{"index": 1, "id": "call_0", "function": {"name": "Read", "arguments": "{\"file_path\":\"/etc/hostname\"}"}}]}}]}));
        t.push(&json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "and\":\"df\"}"}}]}}]}));
        t.push(&json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}));
        t.push(&json!({"choices": [], "usage": {"prompt_tokens": 1200, "completion_tokens": 30, "prompt_tokens_details": {"cached_tokens": 1000}}}));
        let end = t.finish();
        assert_eq!(
            names(&end),
            [
                "content_block_stop",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        assert_eq!(end[1].1["content_block"]["name"], "Bash");
        assert_eq!(end[1].1["index"], 1);
        assert_eq!(end[2].1["delta"]["partial_json"], "{\"command\":\"df\"}");
        assert_eq!(end[1].1["content_block"]["id"], "call_a1b2c3d4e5");
        // "call_0" could be any call of any answer: it gets an id of its own.
        let renamed = end[4].1["content_block"]["id"].as_str().unwrap();
        assert!(
            renamed.starts_with("call_") && renamed.len() > 12 && renamed != call_id("call_0"),
            "{renamed}"
        );
        assert_eq!(end[7].1["delta"]["stop_reason"], "tool_use");
        assert_eq!(
            end[7].1["usage"],
            json!({"input_tokens": 200, "output_tokens": 30, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0})
        );
        assert!(t.finish().is_empty());
    }

    #[test]
    fn a_whole_completion_as_a_message() {
        let chat = json!({"id": "chatcmpl-7", "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "21 GB are free."}}],
            "usage": {"prompt_tokens": 50, "completion_tokens": 6}});
        let m = message(&chat, "gpt-test");
        assert_eq!(
            m["content"],
            json!([{"type": "text", "text": "21 GB are free."}])
        );
        assert_eq!(m["stop_reason"], "end_turn");
        assert_eq!(m["usage"]["input_tokens"], 50);
        let mut t = StreamTranslator::new("gpt-test");
        let ev = t.whole(&chat);
        assert_eq!(
            names(&ev),
            [
                "message_start",
                "ping",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
    }

    #[test]
    fn error_bodies_of_several_shapes() {
        assert_eq!(
            error_message(
                r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#
            ),
            "Incorrect API key provided"
        );
        assert_eq!(
            error_message(r#"{"error":"model not found"}"#),
            "model not found"
        );
        assert_eq!(
            error_message("<html>502 Bad Gateway</html>"),
            "<html>502 Bad Gateway</html>"
        );
        assert_eq!(error_type(401), "authentication_error");
        assert_eq!(error_type(503), "api_error");
    }
}
