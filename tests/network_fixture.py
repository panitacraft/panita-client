"""Loopback-only browser fixture. Never supply real OAuth data or trust its certificate."""
import argparse
import ssl
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent / "fixtures" / "network"


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        # Every path returns 404. No JavaScript, deep links, redirects or echo of input.
        body = b"<!doctype html><title>Panita isolated 404 fixture</title><h1>404 - Synthetic callback unavailable</h1><p>This test cannot complete a launcher login.</p>"
        self.send_response(404)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("Content-Security-Policy", "default-src 'none'; frame-ancestors 'none'")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):
        # Do not log URLs, fragments, parameters or request headers.
        pass


def servers():
    http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    https = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(FIXTURES / "localhost-cert.pem", FIXTURES / "localhost-key.pem")
    https.socket = context.wrap_socket(https.socket, server_side=True)
    return http, https


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=int, default=300, help="Lifetime, 1-3600 seconds (default: 300)")
    args = parser.parse_args()
    if not 1 <= args.seconds <= 3600:
        parser.error("--seconds must be between 1 and 3600")
    http, https = servers()
    for server in (http, https):
        threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f"HTTP 404: http://127.0.0.1:{http.server_port}/callback#code=fixture-code&state=fixture-state", flush=True)
    print(f"Untrusted TLS: https://127.0.0.1:{https.server_port}/callback", flush=True)
    print("Use only synthetic data. Stop at the certificate warning; do not bypass it.", flush=True)
    print(f"Stops automatically after {args.seconds} seconds; Ctrl+C also stops it.", flush=True)
    try:
        threading.Event().wait(args.seconds)
    except KeyboardInterrupt:
        pass
    finally:
        for server in (http, https):
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    main()
