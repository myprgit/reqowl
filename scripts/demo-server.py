#!/usr/bin/env python3
"""Tiny API for examples/demo. Usage: python3 scripts/demo-server.py [port]"""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

TOKEN = "demo-token-123"
ORDERS = [{"id": i, "item": f"seed #{i}"} for i in range(1, 6)]


class Handler(BaseHTTPRequestHandler):
    def send_json(self, status, data):
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def authed(self):
        if self.headers.get("Authorization") == f"Bearer {TOKEN}":
            return True
        self.send_json(401, {"error": "missing or wrong bearer token"})
        return False

    def read_json(self):
        n = int(self.headers.get("Content-Length") or 0)
        try:
            return json.loads(self.rfile.read(n) or b"{}")
        except json.JSONDecodeError:
            return None

    def do_GET(self):
        url = urlparse(self.path)
        if url.path == "/api/orders":
            if self.authed():
                limit = int(parse_qs(url.query).get("limit", ["10"])[0])
                self.send_json(200, ORDERS[:limit])
        else:
            self.send_json(404, {"error": f"no route {url.path}"})

    def do_POST(self):
        url = urlparse(self.path)
        data = self.read_json()
        if data is None:
            self.send_json(400, {"error": "body is not JSON"})
        elif url.path == "/api/login":
            if data.get("password") == "hoot":
                self.send_json(200, {"token": TOKEN, "user": data.get("user")})
            else:
                self.send_json(403, {"error": "wrong password"})
        elif url.path == "/api/graphql":
            if self.authed():
                self.send_json(200, {"data": {"me": {"name": "owl", "orders": len(ORDERS)}}})
        else:
            self.send_json(404, {"error": f"no route {url.path}"})


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8787
    print(f"demo API on http://127.0.0.1:{port}/api")
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
