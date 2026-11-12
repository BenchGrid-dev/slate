//! The gateway end to end: a Messages API client on one side, a scripted endpoint on
//! the other, real sockets in between.

use serde_json::{json, Value};
use slate_gateway::{Api, Auth, Format, Gateway};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

/// What the endpoint received: (path, headers lower-cased, body).
type Seen = (String, Vec<(String, String)>, Value);

/// An endpoint that answers its requests with `replies`, in order: (status, content type, body).
fn endpoint(replies: Vec<(u16, &'static str, String)>) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (status, content_type, body) in replies {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut headers = vec![];
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                let (k, v) = h.trim().split_once(':').unwrap();
                headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
            let length = headers
                .iter()
                .find(|(k, _)| k == "content-length")
                .and_then(|(_, v)| v.parse::<usize>().ok())
                .unwrap_or(0);
            let mut raw = vec![0u8; length];
            reader.read_exact(&mut raw).unwrap();
            tx.send((
                path,
                headers,
                serde_json::from_slice(&raw).unwrap_or(Value::Null),
            ))
            .unwrap();
            let mut stream = reader.into_inner();
            write!(stream, "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    (url, rx)
}

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.1
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn api(format: Format, base_url: String, model: Option<&str>) -> Api {
    Api {
        format,
        base_url,
        api_key: "sk-real-key".into(),
        model: model.map(Into::into),
        small_model: None,
        auth: Auth::Auto,
    }
}

/// POST a Messages API request to the gateway: (status, body).
fn ask(gateway: &Gateway, token: &str, body: &Value) -> (u16, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .post(format!("{}/v1/messages", gateway.base_url()))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "tools-2099")
        .send(body.to_string())
        .unwrap();
    (
        response.status().as_u16(),
        response.body_mut().read_to_string().unwrap(),
    )
}

/// The events of a Messages API stream: (name, data).
fn events(body: &str) -> Vec<(String, Value)> {
    body.split("\n\n")
        .filter_map(|block| {
            let name = block.lines().find_map(|l| l.strip_prefix("event: "))?;
            let data = block.lines().find_map(|l| l.strip_prefix("data: "))?;
            Some((name.to_string(), serde_json::from_str(data).ok()?))
        })
        .collect()
}

fn request() -> Value {
    json!({
        "model": "claude-sonnet-4-5", "max_tokens": 32000, "stream": true,
        "system": "You operate this machine.",
        "tools": [{"name": "Bash", "description": "Run a command", "input_schema": {"type": "object"}}],
        "messages": [{"role": "user", "content": "how much disk is free?"}],
    })
}

#[test]
fn an_openai_endpoint_is_asked_in_its_own_format_and_answers_as_a_message_stream() {
    let chunks = [
        json!({"id": "chatcmpl-1", "choices": [{"delta": {"role": "assistant", "content": "Let me look."}}]}),
        json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_9f8e7d6c5b4a", "function": {"name": "Bash", "arguments": "{\"command\":\"df -h /\"}"}}]}}]}),
        json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
        json!({"choices": [], "usage": {"prompt_tokens": 900, "completion_tokens": 21}}),
    ];
    let stream: String = chunks
        .iter()
        .map(|c| format!("data: {c}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n";
    let refusal = json!({"error": {"message": "Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead.", "type": "invalid_request_error"}});
    let (url, seen) = endpoint(vec![
        (400, "application/json", refusal.to_string()),
        (200, "text/event-stream", stream),
    ]);
    let gateway =
        Gateway::start(api(Format::Openai, format!("{url}/v1"), Some("gpt-test"))).unwrap();

    let (status, body) = ask(&gateway, gateway.token(), &request());
    assert_eq!(status, 200, "{body}");
    let ev = events(&body);
    let names: Vec<&str> = ev.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "message_start",
            "ping",
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
    assert_eq!(ev[0].1["message"]["model"], "gpt-test");
    assert_eq!(ev[3].1["delta"]["text"], "Let me look.");
    assert_eq!(
        ev[5].1["content_block"],
        json!({"type": "tool_use", "id": "call_9f8e7d6c5b4a", "name": "Bash", "input": {}})
    );
    assert_eq!(
        ev[6].1["delta"]["partial_json"],
        "{\"command\":\"df -h /\"}"
    );
    assert_eq!(ev[8].1["delta"]["stop_reason"], "tool_use");
    assert_eq!(ev[8].1["usage"]["input_tokens"], 900);

    // The first try as the CLI asked; the second with what the refusal named.
    let first = seen.recv().unwrap();
    assert_eq!(first.0, "/v1/chat/completions");
    assert_eq!(header(&first, "authorization"), Some("Bearer sk-real-key"));
    assert_eq!(first.2["model"], "gpt-test");
    assert_eq!(first.2["max_tokens"], 32000);
    assert_eq!(
        first.2["messages"][0],
        json!({"role": "system", "content": "You operate this machine."})
    );
    assert_eq!(first.2["tools"][0]["function"]["name"], "Bash");
    let second = seen.recv().unwrap();
    assert_eq!(second.2["max_completion_tokens"], 32000);
    assert!(second.2.get("max_tokens").is_none());
}

#[test]
fn the_gateway_only_answers_its_own_session() {
    let (url, _seen) = endpoint(vec![]);
    let gateway = Gateway::start(api(Format::Openai, url, Some("m"))).unwrap();
    let (status, body) = ask(&gateway, "sk-real-key", &request());
    assert_eq!(status, 401, "{body}");
    assert!(gateway.token().starts_with("slate-session-") && !gateway.base_url().contains("sk-"));
}

#[test]
fn an_endpoint_s_refusal_reaches_the_cli_in_its_own_terms() {
    let refusal = json!({"error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}});
    let (url, _seen) = endpoint(vec![(401, "application/json", refusal.to_string())]);
    let gateway = Gateway::start(api(Format::Openai, url, Some("m"))).unwrap();
    let (status, body) = ask(&gateway, gateway.token(), &request());
    // Not 401: the CLI would retry that for minutes, as if its own sign-in had lapsed.
    assert_eq!(status, 400);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["type"], "authentication_error");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Incorrect API key provided"));
    assert!(gateway.last_error().unwrap().contains("Incorrect API key"));
}

#[test]
fn an_anthropic_endpoint_gets_the_request_as_it_came_with_the_real_key() {
    let stream = "event: message_start\ndata: {\"type\":\"message_start\"}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let (url, seen) = endpoint(vec![
        (200, "text/event-stream", stream.to_string()),
        (200, "text/event-stream", stream.to_string()),
    ]);
    // Without a model the CLI's choice stands; with one, it is asked for instead.
    let gateway = Gateway::start(api(Format::Anthropic, url.clone(), None)).unwrap();
    let (status, body) = ask(&gateway, gateway.token(), &request());
    assert_eq!((status, body.as_str()), (200, stream));
    let first = seen.recv().unwrap();
    assert_eq!(first.0, "/v1/messages");
    assert_eq!(header(&first, "x-api-key"), Some("sk-real-key"));
    assert_eq!(header(&first, "anthropic-beta"), Some("tools-2099"));
    assert_eq!(first.2["model"], "claude-sonnet-4-5");
    assert_eq!(first.2["system"], "You operate this machine.");

    gateway.set_model(Some("kimi-test".into()));
    ask(&gateway, gateway.token(), &request());
    assert_eq!(seen.recv().unwrap().2["model"], "kimi-test");
}

#[test]
fn a_connection_can_be_tried() {
    let answer = json!({"id": "chatcmpl-2", "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "OK"}}],
        "usage": {"prompt_tokens": 12, "completion_tokens": 1}});
    let (url, seen) = endpoint(vec![(200, "application/json", answer.to_string())]);
    let result = slate_gateway::test(&api(Format::Openai, url, Some("gpt-test")));
    assert!(result.ok, "{}", result.message);
    assert!(
        result.message.contains("gpt-test") && result.message.contains("OK"),
        "{}",
        result.message
    );
    assert_eq!(seen.recv().unwrap().2["stream"], Value::Null);

    let refused = slate_gateway::test(&api(Format::Openai, "http://127.0.0.1:9".into(), Some("m")));
    assert!(!refused.ok);
    assert_eq!(refused.status, Some(502));
}
