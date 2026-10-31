"""A tiny web server for the shop. Reads PORT and APP_ENV from the environment."""

import http.server
import os
import time

from shop import PRODUCTS, format_price

PORT = int(os.environ.get("PORT", "8000"))
ENV = os.environ.get("APP_ENV", "dev")
LOG_DIR = os.environ.get("LOG_DIR", "logs")
os.makedirs(LOG_DIR, exist_ok=True)


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        rows = "".join(f"<li>{name}: {format_price(cents)}</li>" for name, cents in PRODUCTS)
        body = f"<h1>Tiny Shop ({ENV})</h1><ul>{rows}</ul>".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        line = f"{time.strftime('%H:%M:%S')} {self.address_string()} {fmt % args}"
        print(line, flush=True)
        with open(os.path.join(LOG_DIR, "server.log"), "a") as f:
            f.write(line + "\n")


print(f"Tiny Shop ({ENV}) listening on http://localhost:{PORT}", flush=True)
http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
