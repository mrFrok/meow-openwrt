#!/usr/bin/env python3
"""Compare meow's REST API with mihomo's on the calls OpenWrt front-ends make.

usage: api-compare.py MEOW_BIN MIHOMO_BIN [WORKDIR]

Starts both cores on the same config (different ports), sends the same
requests to each and prints, per request, the status codes and the JSON
*shape* differences (keys and value types; values themselves differ
naturally). Streaming endpoints are compared on their first JSON line.
"""
import http.server
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request

SECRET = "s3cret"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class NoContent(http.server.BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 (stdlib name)
        self.send_response(204)
        self.end_headers()

    def log_message(self, *_):
        pass


def config(probe_url, mixed_port):
    return f"""mixed-port: {mixed_port}
mode: rule
log-level: info
secret: {SECRET}
proxies:
  - {{name: socks-a, type: socks5, server: 127.0.0.1, port: 1}}
proxy-groups:
  - {{name: Sel, type: select, proxies: [DIRECT, socks-a, REJECT]}}
  - {{name: Auto, type: url-test, proxies: [DIRECT], url: '{probe_url}', interval: 600}}
  - {{name: FromProv, type: select, use: [prov]}}
proxy-providers:
  prov:
    type: file
    path: ./providers/prov.yaml
    health-check: {{enable: true, url: '{probe_url}', interval: 600}}
rule-providers:
  rs:
    type: file
    behavior: domain
    path: ./providers/rs.yaml
rules:
  - RULE-SET,rs,Sel
  - AND,((NETWORK,udp),(RULE-SET,rs)),Auto
  - DOMAIN-SUFFIX,example.org,Sel
  - MATCH,DIRECT
"""


def setup_home(root, name, probe_url):
    home = os.path.join(root, name)
    os.makedirs(os.path.join(home, "providers"), exist_ok=True)
    with open(os.path.join(home, "providers", "prov.yaml"), "w") as f:
        f.write("proxies:\n  - {name: prov-1, type: socks5, server: 127.0.0.1, port: 2}\n")
    with open(os.path.join(home, "providers", "rs.yaml"), "w") as f:
        f.write("payload:\n  - '+.example.com'\n")
    cfg = os.path.join(home, "config.yaml")
    with open(cfg, "w") as f:
        f.write(config(probe_url, free_port()))
    return home, cfg


def request(base, method, path, body=None, stream=False):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(base + path, data=data, method=method)
    req.add_header("Authorization", f"Bearer {SECRET}")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            raw = r.readline() if stream else r.read()
            return r.status, raw
    except urllib.error.HTTPError as e:
        return e.code, e.read()
    except Exception as e:  # timeouts, resets
        return f"ERR {type(e).__name__}", b""


def shape(v):
    if isinstance(v, dict):
        return {k: shape(x) for k, x in sorted(v.items())}
    if isinstance(v, list):
        return [shape(v[0])] if v else []
    if isinstance(v, bool):
        return "bool"
    if isinstance(v, (int, float)):
        return "num"
    if v is None:
        return "null"
    return "str"


def parse(raw):
    raw = raw.strip()
    if not raw:
        return "<empty>"
    try:
        return json.loads(raw)
    except ValueError:
        return "<non-json>"


def diff(a, b, path=""):
    """Shape differences between meow (a) and mihomo (b)."""
    out = []
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b)):
            p = f"{path}.{k}" if path else k
            if k not in a:
                out.append(f"  missing in meow: {p}")
            elif k not in b:
                out.append(f"  extra in meow:   {p}")
            else:
                out += diff(a[k], b[k], p)
    elif isinstance(a, list) and isinstance(b, list):
        if a and b:
            out += diff(a[0], b[0], path + "[]")
    elif a != b and not ({a, b} <= {"num", "null"} if isinstance(a, str) and isinstance(b, str) else False):
        out.append(f"  type {path or '<root>'}: meow={json.dumps(a)[:60]} mihomo={json.dumps(b)[:60]}")
    return out


CALLS = [
    ("GET", "/version", None),
    ("GET", "/configs", None),
    ("PATCH", "/configs", {"mode": "global"}),
    ("PATCH", "/configs", {"mode": "rule"}),
    ("GET", "/proxies", None),
    ("GET", "/proxies/Sel", None),
    ("PUT", "/proxies/Sel", {"name": "socks-a"}),
    ("PUT", "/proxies/Sel", {"name": "nope"}),
    ("GET", "/proxies/nope", None),
    ("DELETE", "/proxies/Auto", None),
    ("GET", "/proxies/DIRECT/delay?url={probe}&timeout=2000", None),
    ("GET", "/group/Auto/delay?url={probe}&timeout=2000", None),
    ("GET", "/providers/proxies", None),
    ("GET", "/providers/proxies/prov", None),
    ("PUT", "/providers/proxies/prov", None),
    ("GET", "/providers/proxies/prov/healthcheck", None),
    ("GET", "/providers/rules", None),
    ("PUT", "/providers/rules/rs", None),
    ("GET", "/rules", None),
    ("GET", "/connections", None),
    ("DELETE", "/connections", None),
    ("GET", "/dns/query?name=localhost&type=A", None),
    ("PUT", "/configs?force=true", {"path": "{cfg}"}),
    ("GET", "/logs?level=info", "stream"),
    ("GET", "/traffic", "stream"),
    ("GET", "/memory", "stream"),
]


def wait_up(base, proc, name):
    for _ in range(100):
        if proc.poll() is not None:
            sys.exit(f"{name} exited early")
        if request(base, "GET", "/version")[0] == 200:
            return
        time.sleep(0.1)
    sys.exit(f"{name} controller did not come up")


def main():
    meow_bin, mihomo_bin = sys.argv[1], sys.argv[2]
    root = sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="api-compare-")
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), NoContent)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    probe = f"http://127.0.0.1:{srv.server_address[1]}/generate_204"

    cores = {}
    for name, binary in (("meow", meow_bin), ("mihomo", mihomo_bin)):
        home, cfg = setup_home(root, name, probe)
        port = free_port()
        flag = "--ext-ctl" if name == "meow" else "-ext-ctl"
        log = open(os.path.join(root, f"{name}.log"), "w")
        proc = subprocess.Popen([binary, "-d", home, "-f", cfg, flag, f"127.0.0.1:{port}"],
                                stdout=log, stderr=subprocess.STDOUT)
        cores[name] = (proc, f"http://127.0.0.1:{port}", cfg)
    try:
        for name, (proc, base, _) in cores.items():
            wait_up(base, proc, name)
        time.sleep(1)
        mismatches = 0
        for method, path, body in CALLS:
            res = {}
            for name, (_, base, cfg) in cores.items():
                p = path.replace("{probe}", urllib.request.quote(probe, safe=""))
                b = {"path": cfg} if body == {"path": "{cfg}"} else body
                stream = body == "stream"
                status, raw = request(base, method, p, None if stream else b, stream)
                res[name] = (status, shape(parse(raw)))
            (ms, mshape), (hs, hshape) = res["meow"], res["mihomo"]
            lines = diff(mshape, hshape)
            ok = ms == hs and not lines
            mismatches += not ok
            label = f"{method} {path}" + (f" {json.dumps(body)}" if isinstance(body, dict) else "")
            print(f"{'ok ' if ok else 'DIF'} {label}: meow={ms} mihomo={hs}")
            limit = None if os.environ.get("API_COMPARE_ALL") else 12
            for line in lines[:limit]:
                print(line)
            if limit and len(lines) > limit:
                print(f"  ... {len(lines) - limit} more")
        print(f"\n{mismatches} of {len(CALLS)} calls differ; logs in {root}")
    finally:
        for proc, _, _ in cores.values():
            proc.send_signal(signal.SIGTERM)
        for proc, _, _ in cores.values():
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
        srv.shutdown()
        if len(sys.argv) <= 3:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
