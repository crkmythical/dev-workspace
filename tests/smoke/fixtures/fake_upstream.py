#!/usr/bin/env python3
"""Smoke-test helper (stdlib only): a fake Clash subscription provider AND the proxy node it lists.

  GET /sub.yaml        a provider with one HTTP proxy node: this container, port 8888
  GET /sub-dead.yaml   one node on a closed port: the nodes load, nothing gets out
  GET /sub-empty.yaml  a provider with no nodes at all
  GET /sub-ss.yaml     one Shadowsocks node (only when SS_NODE_IP is set): a real proxy protocol
  CONNECT host:port    (port 8888) the proxy node itself; tunnels to the real internet

Every CONNECT is logged, which is the evidence that traffic really took the Clash path.
Configuration comes from the environment so that several instances can run side by side:
  UPSTREAM_IP   this container's address on the test network (default 172.30.77.10)
  SS_NODE_IP    address of a Shadowsocks server, if /sub-ss.yaml should list one
  SS_PASSWORD   its password
"""

import asyncio
import http.server
import os
import threading

UPSTREAM_IP = os.environ.get("UPSTREAM_IP", "172.30.77.10")
SS_NODE_IP = os.environ.get("SS_NODE_IP", "")
SS_PASSWORD = os.environ.get("SS_PASSWORD", "smoke-ss-password")

PROVIDERS = {
    "/sub.yaml": f"proxies:\n  - {{name: local-http, type: http, server: {UPSTREAM_IP}, port: 8888}}\n",
    "/sub-dead.yaml": f"proxies:\n  - {{name: dead-http, type: http, server: {UPSTREAM_IP}, port: 8899}}\n",
    "/sub-empty.yaml": "proxies: []\n",
}
if SS_NODE_IP:
    PROVIDERS["/sub-ss.yaml"] = (
        "proxies:\n"
        f"  - {{name: ss-node, type: ss, server: {SS_NODE_IP}, port: 8388, "
        f"cipher: chacha20-ietf-poly1305, password: {SS_PASSWORD}}}\n"
    )


class Provider(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = PROVIDERS.get(self.path.split("?")[0])
        if body is None:
            self.send_response(404)
            self.end_headers()
            return
        data = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/yaml")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        print(f"PROVIDER served {self.path}", flush=True)

    def log_message(self, *_):
        pass


async def pipe(reader, writer):
    try:
        while data := await reader.read(65536):
            writer.write(data)
            await writer.drain()
    except Exception:
        pass
    finally:
        try:
            writer.close()
        except Exception:
            pass


async def handle(client_reader, client_writer):
    try:
        head = await client_reader.readuntil(b"\r\n\r\n")
        line = head.split(b"\r\n", 1)[0].decode(errors="replace")
        method, target, *_ = line.split(" ")
        if method != "CONNECT":
            client_writer.write(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
            await client_writer.drain()
            client_writer.close()
            return
        host, _, port = target.rpartition(":")
        print(f"CONNECT {target}", flush=True)
        up_reader, up_writer = await asyncio.open_connection(host, int(port))
        client_writer.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        await client_writer.drain()
        await asyncio.gather(pipe(client_reader, up_writer), pipe(up_reader, client_writer))
    except Exception as err:
        print(f"CONNECT failed: {err}", flush=True)
        try:
            client_writer.close()
        except Exception:
            pass


async def main():
    server = http.server.ThreadingHTTPServer(("0.0.0.0", 8000), Provider)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    proxy = await asyncio.start_server(handle, "0.0.0.0", 8888)
    print("fake provider on :8000, proxy node on :8888", flush=True)
    async with proxy:
        await proxy.serve_forever()


asyncio.run(main())
