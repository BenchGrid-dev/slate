#!/usr/bin/env python3
"""A scripted OpenAI-format endpoint (chat completions) for the gateway tests.

    python3 tests/e2e/mock_openai.py PORT [LOG]

It needs no key and no model: it answers a first request that offers a `Bash` tool and
says "run" with a call of it (`echo gateway-works`), a request whose last message is a
tool result with a sentence quoting that result, and anything else with a greeting.
Every request is appended to LOG as one JSON line: what the endpoint was sent.
"""
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[2] if len(sys.argv) > 2 else None


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(p.get("text", "") for p in content if isinstance(p, dict))
    return ""


def decide(body):
    """(text, tool call or None) for a chat completions request."""
    messages = body.get("messages", [])
    last = messages[-1] if messages else {}
    tools = [t.get("function", {}).get("name") for t in body.get("tools", [])]
    users = [text_of(m.get("content")) for m in messages if m.get("role") == "user"]
    if last.get("role") == "tool":
        return f"The command printed: {text_of(last.get('content')).strip()}", None
    if any(m.get("role") == "tool" for m in messages[-3:]):
        tool = next(m for m in reversed(messages) if m.get("role") == "tool")
        return f"The command printed: {text_of(tool.get('content')).strip()}", None
    if "Bash" in tools and users and "run" in users[-1].lower():
        return "Let me run it.", {"name": "Bash", "arguments": json.dumps({"command": "echo gateway-works", "description": "Print the test line"})}
    return "Hello from the mock model.", None


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        if LOG:
            with open(LOG, "a") as f:
                f.write(json.dumps({"path": self.path, "authorization": self.headers.get("authorization"), "body": body}) + "\n")
        if not self.path.endswith("/chat/completions"):
            return self.reply(404, {"error": {"message": f"no such path {self.path}"}})
        if self.headers.get("authorization") != "Bearer sk-mock":
            return self.reply(401, {"error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}})
        text, call = decide(body)
        usage = {"prompt_tokens": 100, "completion_tokens": 10, "total_tokens": 110}
        if not body.get("stream"):
            message = {"role": "assistant", "content": text}
            if call:
                message["tool_calls"] = [{"id": "call_mock_1", "type": "function", "function": call}]
            return self.reply(200, {"id": "chatcmpl-mock", "object": "chat.completion", "model": body.get("model"),
                                    "choices": [{"index": 0, "message": message, "finish_reason": "tool_calls" if call else "stop"}], "usage": usage})
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()

        def chunk(delta, finish=None, **extra):
            data = {"id": "chatcmpl-mock", "object": "chat.completion.chunk", "model": body.get("model"),
                    "choices": [{"index": 0, "delta": delta, "finish_reason": finish}], **extra}
            self.wfile.write(f"data: {json.dumps(data)}\n\n".encode())
            self.wfile.flush()
            time.sleep(0.02)

        chunk({"role": "assistant", "content": ""})
        for word in text.split(" "):
            chunk({"content": word + " "})
        if call:
            half = len(call["arguments"]) // 2
            chunk({"tool_calls": [{"index": 0, "id": "call_mock_1", "type": "function", "function": {"name": call["name"], "arguments": call["arguments"][:half]}}]})
            chunk({"tool_calls": [{"index": 0, "function": {"arguments": call["arguments"][half:]}}]})
        chunk({}, finish="tool_calls" if call else "stop")
        self.wfile.write(f"data: {json.dumps({'id': 'chatcmpl-mock', 'choices': [], 'usage': usage})}\n\ndata: [DONE]\n\n".encode())
        self.wfile.flush()
        self.close_connection = True

    def reply(self, status, obj):
        data = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
