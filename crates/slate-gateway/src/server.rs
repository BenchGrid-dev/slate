//! The gateway's listener: a small HTTP/1.1 server for one client, the agent CLI.
//! One request per connection; answers are written as they come from the endpoint.

use crate::openai::{self, StreamTranslator};
use crate::{Api, Auth, Format, State};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

const MAX_HEADERS: usize = 64 * 1024;
const MAX_BODY: usize = 96 * 1024 * 1024;

pub(crate) fn serve(listener: TcpListener, state: Arc<State>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        let _ = std::thread::Builder::new()
            .name("slate-gateway-request".into())
            .spawn(move || {
                let _ = handle(stream, &state);
            });
    }
}

struct Request {
    method: String,
    /// Path and query, as sent.
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

fn read_request(reader: &mut BufReader<TcpStream>) -> io::Result<Request> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| bad("empty request"))?
        .to_string();
    let target = parts
        .next()
        .ok_or_else(|| bad("no request target"))?
        .to_string();
    let mut headers = vec![];
    let mut seen = line.len();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 {
            return Err(bad("headers cut short"));
        }
        seen += h.len();
        if seen > MAX_HEADERS {
            return Err(bad("headers too long"));
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let mut request = Request {
        method,
        target,
        headers,
        body: vec![],
    };
    if request
        .header("expect")
        .is_some_and(|e| e.eq_ignore_ascii_case("100-continue"))
    {
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    if request
        .header("transfer-encoding")
        .is_some_and(|t| t.to_ascii_lowercase().contains("chunked"))
    {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size)?;
            let n = usize::from_str_radix(size.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| bad("chunk size"))?;
            if n == 0 {
                break;
            }
            if request.body.len() + n > MAX_BODY {
                return Err(bad("body too large"));
            }
            let at = request.body.len();
            request.body.resize(at + n, 0);
            reader.read_exact(&mut request.body[at..])?;
            let mut crlf = [0u8; 2];
            reader.read_exact(&mut crlf)?;
        }
    } else if let Some(n) = request
        .header("content-length")
        .and_then(|n| n.parse::<usize>().ok())
    {
        if n > MAX_BODY {
            return Err(bad("body too large"));
        }
        request.body.resize(n, 0);
        reader.read_exact(&mut request.body)?;
    }
    Ok(request)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        529 => "Overloaded",
        _ => "Status",
    }
}

fn respond(out: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    write!(out, "HTTP/1.1 {status} {}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", reason(status), body.len())?;
    out.write_all(body)?;
    out.flush()
}

fn respond_json(out: &mut TcpStream, status: u16, body: &Value) -> io::Result<()> {
    respond(out, status, "application/json", body.to_string().as_bytes())
}

/// An error in the Messages API's shape, which the CLI shows to the person.
fn error_body(status: u16, message: &str) -> Value {
    json!({"type": "error", "error": {"type": openai::error_type(status), "message": message}})
}

/// The status the CLI is told. A refused key stays refused however often it is
/// presented, but the CLI treats 401 and 403 as its own sign-in having lapsed and
/// tries again for minutes; as a 400 it reports the endpoint's words at once.
fn status_for_cli(status: u16) -> u16 {
    match status {
        401 | 403 => 400,
        other => other,
    }
}

fn respond_error(out: &mut TcpStream, state: &State, status: u16, message: &str) -> io::Result<()> {
    *state.last_error.lock().unwrap() = Some(message.to_string());
    respond_json(out, status_for_cli(status), &error_body(status, message))
}

fn start_stream(out: &mut TcpStream, status: u16, content_type: &str) -> io::Result<()> {
    // No length: the body ends when the connection closes.
    write!(out, "HTTP/1.1 {status} {}\r\ncontent-type: {content_type}\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n", reason(status))?;
    out.flush()
}

fn send_event(out: &mut TcpStream, name: &str, data: &Value) -> io::Result<()> {
    write!(out, "event: {name}\ndata: {data}\n\n")?;
    out.flush()
}

fn handle(stream: TcpStream, state: &State) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_nodelay(true).ok();
    let mut reader = BufReader::new(stream);
    let request = match read_request(&mut reader) {
        Ok(r) => r,
        Err(e) => {
            let mut out = reader.into_inner();
            return respond_json(
                &mut out,
                400,
                &error_body(400, &format!("slate gateway: {e}")),
            );
        }
    };
    let mut out = reader.into_inner();
    if request.path() == "/" && matches!(request.method.as_str(), "GET" | "HEAD") {
        return respond(&mut out, 200, "text/plain", b"slate gateway\n");
    }
    let presented = request.header("x-api-key").or_else(|| {
        request
            .header("authorization")
            .map(|a| a.trim_start_matches("Bearer ").trim())
    });
    if presented != Some(state.token.as_str()) {
        return respond_json(
            &mut out,
            401,
            &error_body(
                401,
                "slate gateway: this address is for the session that started it",
            ),
        );
    }
    let api = state.api.read().unwrap().clone();
    match api.format {
        Format::Anthropic => pass_through(&mut out, state, &api, request),
        Format::Openai => match (request.method.as_str(), request.path()) {
            ("POST", "/v1/messages") => chat(&mut out, state, &api, &request),
            ("POST", "/v1/messages/count_tokens") => {
                let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                respond_json(
                    &mut out,
                    200,
                    &json!({"input_tokens": openai::estimate_tokens(&body)}),
                )
            }
            _ => respond_json(
                &mut out,
                404,
                &error_body(
                    404,
                    "slate gateway: an OpenAI-format connection only serves messages",
                ),
            ),
        },
    }
}

fn unreachable(api: &Api, error: &ureq::Error) -> String {
    format!("could not reach {}: {error}", api.host())
}

/// An Anthropic-format endpoint: the request goes on as it came, with the real key.
fn pass_through(out: &mut TcpStream, state: &State, api: &Api, request: Request) -> io::Result<()> {
    let url = api.anthropic_url(&request.target);
    let mut body = request.body.clone();
    let has_model = api.model.as_deref().is_some_and(|m| !m.trim().is_empty());
    if has_model && request.method == "POST" && request.path().starts_with("/v1/messages") {
        if let Ok(mut v) = serde_json::from_slice::<Value>(&body) {
            if let Some(asked) = v.get("model").and_then(Value::as_str).map(str::to_string) {
                v["model"] = json!(api.model_for(&asked));
                body = v.to_string().into_bytes();
            }
        }
    }
    let mut builder = ureq::http::Request::builder()
        .method(request.method.as_str())
        .uri(&url);
    for name in [
        "content-type",
        "accept",
        "anthropic-beta",
        "anthropic-dangerous-direct-browser-access",
    ] {
        if let Some(v) = request.header(name) {
            builder = builder.header(name, v);
        }
    }
    builder = builder.header(
        "anthropic-version",
        request.header("anthropic-version").unwrap_or("2023-06-01"),
    );
    let key = api.api_key.trim();
    let official = api.host().ends_with("anthropic.com");
    if matches!(api.auth, Auth::XApiKey) || (api.auth == Auth::Auto) {
        builder = builder.header("x-api-key", key);
    }
    if matches!(api.auth, Auth::Bearer) || (api.auth == Auth::Auto && !official) {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    let sent = if matches!(request.method.as_str(), "GET" | "HEAD" | "DELETE") {
        builder
            .body(())
            .map_err(|e| bad(&e.to_string()))
            .map(|r| state.agent.run(r))
    } else {
        builder
            .body(body)
            .map_err(|e| bad(&e.to_string()))
            .map(|r| state.agent.run(r))
    };
    let response = match sent {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return respond_error(out, state, 502, &unreachable(api, &e)),
        Err(e) => return respond_error(out, state, 400, &format!("slate gateway: {e}")),
    };
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let mut upstream = response.into_body().into_reader();
    if status >= 400 {
        let mut text = String::new();
        let _ = (&mut upstream).take(64 * 1024).read_to_string(&mut text);
        *state.last_error.lock().unwrap() = Some(format!(
            "{} answered {status}: {}",
            api.host(),
            openai::error_message(&text)
        ));
        return respond(out, status_for_cli(status), &content_type, text.as_bytes());
    }
    start_stream(out, status, &content_type)?;
    let mut buffer = [0u8; 8192];
    loop {
        match upstream.read(&mut buffer) {
            Ok(0) | Err(_) => return Ok(()),
            Ok(n) => {
                out.write_all(&buffer[..n])?;
                out.flush()?;
            }
        }
    }
}

/// An OpenAI-format endpoint: the request is rewritten, and so is the answer.
fn chat(out: &mut TcpStream, state: &State, api: &Api, request: &Request) -> io::Result<()> {
    let body: Value = match serde_json::from_slice(&request.body) {
        Ok(v) => v,
        Err(e) => {
            return respond_error(
                out,
                state,
                400,
                &format!("slate gateway: the request is not JSON: {e}"),
            )
        }
    };
    let model = api.model_for(body.get("model").and_then(Value::as_str).unwrap_or(""));
    let asked_max = body.get("max_tokens").and_then(Value::as_u64);
    let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let mut attempts = 0;
    let response = loop {
        let quirks = state.quirks.lock().unwrap().clone();
        let chat = openai::chat_request(&body, &model, &quirks);
        let sent = state
            .agent
            .post(api.chat_url())
            .header("content-type", "application/json")
            .header(
                "accept",
                if streaming {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .header("authorization", format!("Bearer {}", api.api_key.trim()))
            .send(chat.to_string());
        let mut response = match sent {
            Ok(r) => r,
            Err(e) => return respond_error(out, state, 502, &unreachable(api, &e)),
        };
        let status = response.status().as_u16();
        if status < 400 {
            break response;
        }
        let text = response.body_mut().read_to_string().unwrap_or_default();
        let message = openai::error_message(&text);
        // A parameter this provider or model does not take: drop it and ask again.
        if status == 400 && attempts < 3 && state.quirks.lock().unwrap().learn(&message, asked_max)
        {
            attempts += 1;
            continue;
        }
        return respond_error(
            out,
            state,
            status,
            &format!("{} answered {status}: {message}", api.host()),
        );
    };
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut upstream = BufReader::new(response.into_body().into_reader());
    if !streaming {
        let mut text = String::new();
        upstream.read_to_string(&mut text).ok();
        return match serde_json::from_str::<Value>(&text) {
            Ok(v) => respond_json(out, 200, &openai::message(&v, &model)),
            Err(_) => respond_error(
                out,
                state,
                502,
                &format!(
                    "{} sent something that is not a chat completion",
                    api.host()
                ),
            ),
        };
    }
    start_stream(out, 200, "text/event-stream")?;
    let mut translator = StreamTranslator::new(&model);
    if !content_type.contains("event-stream") {
        // A stream was asked for and a whole answer came: some servers do that.
        let mut text = String::new();
        upstream.read_to_string(&mut text).ok();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        for (name, data) in translator.whole(&v) {
            send_event(out, name, &data)?;
        }
        return Ok(());
    }
    let mut line = String::new();
    loop {
        line.clear();
        match upstream.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                let message = format!("the answer from {} broke off: {e}", api.host());
                *state.last_error.lock().unwrap() = Some(message.clone());
                return send_event(out, "error", &error_body(502, &message));
            }
        }
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(error) = chunk.get("error").filter(|e| !e.is_null()) {
            let message = format!(
                "{}: {}",
                api.host(),
                openai::error_message(&json!({"error": error}).to_string())
            );
            *state.last_error.lock().unwrap() = Some(message.clone());
            return send_event(out, "error", &error_body(502, &message));
        }
        for (name, data) in translator.push(&chunk) {
            send_event(out, name, &data)?;
        }
    }
    for (name, data) in translator.finish() {
        send_event(out, name, &data)?;
    }
    Ok(())
}
