#!/usr/bin/env python3
import argparse
import json
import os
import socket
import sys
import time

PROTOCOL = "fast-explorer/1"


def request(socket_path: str, method: str, params=None, timeout=5.0) -> dict:
    payload = {
        "protocol": PROTOCOL,
        "id": 1,
        "method": method,
        "params": params or {},
    }
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(timeout)
        client.connect(socket_path)
        client.sendall((json.dumps(payload) + "\n").encode())
        stream = client.makefile("r", encoding="utf-8")
        line = stream.readline()
        if not line:
            raise RuntimeError("FastExplorer IPC closed without a response")
        try:
            response = json.loads(line)
        except json.JSONDecodeError as error:
            raise RuntimeError(f"invalid IPC response: {error}") from error
    if not isinstance(response, dict):
        raise RuntimeError("FastExplorer IPC returned a non-object JSON response")
    if not response.get("ok"):
        error = response.get("error") or {}
        if isinstance(error, dict):
            message = error.get("message", "IPC request failed")
        else:
            message = f"IPC request failed: {error}"
        raise RuntimeError(message)
    return response


def wait_until_ready(socket_path: str, timeout: float) -> dict:
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            remaining = max(0.1, deadline - time.monotonic())
            return request(socket_path, "ping", timeout=remaining)
        except (OSError, RuntimeError) as error:
            last_error = error
            time.sleep(0.05)
    raise TimeoutError(f"FastExplorer IPC did not become ready: {last_error}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="FastExplorer IPC helper")
    parser.add_argument("--socket", required=True, help="Unix-domain socket path")
    parser.add_argument(
        "command",
        choices=["wait", "ping", "navigate", "state", "search", "clear-search", "refresh", "new-tab"],
    )
    parser.add_argument("value", nargs="?", help="path for navigate or query for search")
    parser.add_argument("--timeout", type=float, default=15.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.command == "wait":
            response = wait_until_ready(args.socket, args.timeout)
        elif args.command == "navigate":
            if not args.value:
                raise ValueError("navigate requires a path")
            response = request(args.socket, "navigate", {"path": os.path.abspath(args.value)})
        elif args.command == "search":
            if args.value is None:
                raise ValueError("search requires a query")
            response = request(args.socket, "search", {"query": args.value})
        else:
            method = {
                "ping": "ping",
                "state": "get_state",
                "clear-search": "clear_search",
                "refresh": "refresh",
                "new-tab": "new_tab",
            }[args.command]
            response = request(args.socket, method)
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(json.dumps(response, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
