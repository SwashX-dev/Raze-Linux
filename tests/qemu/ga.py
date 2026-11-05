#!/usr/bin/env python3
"""Run a command in the guest through qemu-guest-agent and print its output.
usage: ga.py SOCK 'shell command' [timeout_s]"""
import base64, json, socket, sys, time
sock, cmd = sys.argv[1], sys.argv[2]
timeout = float(sys.argv[3]) if len(sys.argv) > 3 else 120
s = socket.socket(socket.AF_UNIX); s.connect(sock); f = s.makefile("rw")
def call(obj):
    f.write(json.dumps(obj) + "\n"); f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r:
            if "error" in r: raise SystemExit(r["error"])
            return r["return"]
call({"execute": "guest-sync", "arguments": {"id": 4242}})
pid = call({"execute": "guest-exec", "arguments": {"path": "/run/current-system/sw/bin/bash", "arg": ["-lc", cmd], "capture-output": True}})["pid"]
t0 = time.time()
while True:
    st = call({"execute": "guest-exec-status", "arguments": {"pid": pid}})
    if st.get("exited"):
        for k in ("out-data", "err-data"):
            if k in st: sys.stdout.write(base64.b64decode(st[k]).decode("utf-8", "replace"))
        sys.exit(st.get("exitcode", 0))
    if time.time() - t0 > timeout: raise SystemExit("timeout")
    time.sleep(0.2)
