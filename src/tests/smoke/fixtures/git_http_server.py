#!/usr/bin/env python3
"""Smoke-test helper (stdlib + git): a git smart-HTTP server that wants a token the way GitHub does.

`git http-backend` does the git part. In front of it, HTTP Basic: the token is the user name and the
password can be anything, including empty. A request without credentials gets 401 and a Basic
challenge, which is what makes git go and ask for them (or use those in the URL).

  GIT_TOKEN   the accepted user name (required)
  GIT_ROOT    directory holding the bare repositories (default /srv/git)

Every request is logged with what it presented, so a test can tell "git sent the token at once"
from "git asked for a password first".
"""

import base64
import http.server
import os
import subprocess

TOKEN = os.environ["GIT_TOKEN"]
ROOT = os.environ.get("GIT_ROOT", "/srv/git")


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        self.serve()

    def do_POST(self):
        self.serve()

    def log_message(self, *_):
        pass

    def credentials(self):
        header = self.headers.get("Authorization", "")
        if not header.startswith("Basic "):
            return None
        try:
            return base64.b64decode(header[6:]).decode().partition(":")[::2]
        except Exception:
            return None

    def body(self):
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
            data = b""
            while True:
                size = int(self.rfile.readline().strip() or b"0", 16)
                if size == 0:
                    self.rfile.readline()
                    return data
                data += self.rfile.read(size)
                self.rfile.readline()
        length = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(length) if length else b""

    def serve(self):
        creds = self.credentials()
        who = "none" if creds is None else ("token" if creds[0] == TOKEN else "wrong-user")
        print(f"{self.command} {self.path} auth={who}", flush=True)
        payload = self.body()
        if creds is None or creds[0] != TOKEN:
            self.send_response(401)
            self.send_header("WWW-Authenticate", 'Basic realm="git"')
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        path, _, query = self.path.partition("?")
        env = {
            "PATH": os.environ["PATH"],
            "GIT_PROJECT_ROOT": ROOT,
            "GIT_HTTP_EXPORT_ALL": "1",
            "REQUEST_METHOD": self.command,
            "PATH_INFO": path,
            "QUERY_STRING": query,
            "CONTENT_TYPE": self.headers.get("Content-Type", ""),
            "CONTENT_LENGTH": str(len(payload)),
            "REMOTE_USER": creds[0],
            "HTTP_CONTENT_ENCODING": self.headers.get("Content-Encoding", ""),
        }
        result = subprocess.run(["git", "http-backend"], input=payload, env=env, capture_output=True)
        head, _, out = result.stdout.partition(b"\r\n\r\n")
        status = 200
        headers = []
        for line in head.split(b"\r\n"):
            name, _, value = line.partition(b": ")
            if name.lower() == b"status":
                status = int(value.split()[0])
            elif name:
                headers.append((name.decode(), value.decode()))
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)


if __name__ == "__main__":
    http.server.ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
