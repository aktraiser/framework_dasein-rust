#!/usr/bin/env python3
"""A small MCP server over stdio with tools a real model can understand.

Used by the live test of agents discovering MCP tools: the tools have proper
descriptions and schemas, and what they answer cannot be guessed. Every call
received is appended, as one JSON line, to the file given as first argument.
"""
import json
import sys

TOOLS = [
    {
        "name": "get_order_status",
        "description": "Look up where a customer order currently is.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "order_id": {"type": "string", "description": "The order number, digits only"}
            },
            "required": ["order_id"],
        },
    },
    {
        "name": "list_warehouses",
        "description": "List the names of the company's warehouses.",
        "inputSchema": {"type": "object", "properties": {}},
    },
]


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def call(name, arguments):
    if name == "get_order_status":
        order = arguments.get("order_id", "")
        return f"Order {order} is held at customs in Reykjavik, reference BLUE-PALLET-88."
    if name == "list_warehouses":
        return "Lyon, Gdansk, Porto"
    return None


def main():
    log = sys.argv[1] if len(sys.argv) > 1 else None
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        message = json.loads(line)
        method, request_id = message.get("method"), message.get("id")
        if method is None or request_id is None:
            continue
        params = message.get("params") or {}
        if method == "initialize":
            result = {
                "protocolVersion": params.get("protocolVersion"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "orders", "version": "1"},
            }
        elif method == "tools/list":
            result = {"tools": TOOLS}
        elif method == "tools/call":
            name, arguments = params.get("name"), params.get("arguments") or {}
            if log:
                with open(log, "a", encoding="utf-8") as calls:
                    calls.write(json.dumps({"name": name, "arguments": arguments}) + "\n")
            text = call(name, arguments)
            if text is None:
                error = {"code": -32602, "message": "Unknown tool"}
                send({"jsonrpc": "2.0", "id": request_id, "error": error})
                continue
            result = {"content": [{"type": "text", "text": text}]}
        else:
            error = {"code": -32601, "message": "Method not found"}
            send({"jsonrpc": "2.0", "id": request_id, "error": error})
            continue
        send({"jsonrpc": "2.0", "id": request_id, "result": result})


if __name__ == "__main__":
    main()
