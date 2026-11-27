#!/usr/bin/env python3
"""Minimal MCP server over stdio, used by the client's integration tests.

It speaks newline-delimited JSON-RPC and deliberately does the awkward things
real servers do: a banner on stdout, logs on stderr, a notification and a
server-to-client request in the middle of a call, paginated tool listings.
"""
import json
import os
import sys
import time

TOOLS = [
    {"name": "echo", "description": "Echo the input", "inputSchema": {"type": "object"}},
    {"name": "query-docs", "description": "Ends */ a comment", "inputSchema": {"type": "object"}},
    {"name": "fail", "inputSchema": {"type": "object"}},
    {"name": "resource", "inputSchema": {"type": "object"}},
    {"name": "env", "inputSchema": {"type": "object"}},
    {"name": "slow", "inputSchema": {"type": "object"}},
    {"name": "exit", "inputSchema": {"type": "object"}},
]


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def text(value, **extra):
    return {"content": [{"type": "text", "text": value}], **extra}


def call(name, arguments, request_id):
    if name == "echo":
        # Interleave traffic the client must route around before the answer.
        send({"jsonrpc": "2.0", "method": "notifications/message", "params": {"data": "working"}})
        send({"jsonrpc": "2.0", "id": "srv-1", "method": "roots/list"})
        return text(arguments.get("text", ""))
    if name == "fail":
        return text("it broke", isError=True)
    if name == "resource":
        return {
            "content": [
                {"type": "resource", "resource": {"uri": "file:///a.txt", "mimeType": "text/plain", "text": "from a file"}},
                {"type": "resource_link", "uri": "file:///b.txt", "name": "b"},
                {"type": "hologram", "payload": 1},
            ],
            "structuredContent": {"count": 3},
        }
    if name == "env":
        return text(os.environ.get(arguments.get("name", ""), "<unset>"))
    if name == "slow":
        time.sleep(5)
        return text("late")
    if name == "exit":
        sys.exit(0)
    return None


def main():
    print("mcp test server starting")  # a banner that is not protocol
    sys.stdout.flush()
    print("log line on stderr", file=sys.stderr)
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        message = json.loads(line)
        method, request_id = message.get("method"), message.get("id")
        if method is None or request_id is None:
            continue  # a response to our own request, or a notification
        params = message.get("params") or {}
        if method == "initialize":
            result = {
                "protocolVersion": params.get("protocolVersion"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "fixture", "version": "1"},
            }
        elif method == "tools/list":
            # Two pages, to exercise pagination.
            if params.get("cursor") == "page-2":
                result = {"tools": TOOLS[3:]}
            else:
                result = {"tools": TOOLS[:3], "nextCursor": "page-2"}
        elif method == "tools/call":
            result = call(params.get("name"), params.get("arguments") or {}, request_id)
            if result is None:
                send({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32602, "message": "Unknown tool"}})
                continue
        else:
            send({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "Method not found"}})
            continue
        send({"jsonrpc": "2.0", "id": request_id, "result": result})


if __name__ == "__main__":
    main()
