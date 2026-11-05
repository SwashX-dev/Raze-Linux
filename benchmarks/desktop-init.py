#!/usr/bin/env python3
"""Desktop boot benchmark: the same Raze Desktop system booted with different PID 1s.

usage: desktop-init.py OUT.json NAME=VM_RESULT [NAME=VM_RESULT ...] [--boots N]

Each VM_RESULT is a `system.build.vm` output of the same configuration differing only
in `raze.init`. Boots are interleaved (a, b, a, b, ...) after one discarded warm-up
boot per variant (it creates the disk image and machine-id). Milestones are read
inside the guest from /proc, so no instrumentation differs between variants:

  greeter     kernel start -> greetd process started (login screen available)
  network     kernel start -> NetworkManager process started
  settled     memory and process counts 20 s after the greeter started

The guest agent (virtio-serial port org.qemu.guest_agent.0) runs the probes.
"""
import json
import os
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import base64

HERE = os.path.dirname(os.path.abspath(__file__))

PROBE = r"""
tck=$(getconf CLK_TCK)
start() { p=$(pgrep -o -x "$1") || return; awk -v t=$tck '{printf "%.3f", $22 / t}' /proc/$p/stat; }
echo "greeter=$(start greetd)"
echo "network=$(start NetworkManager)"
echo "pid1=$(cat /proc/1/comm)"
"""

SETTLED = r"""
awk '/MemTotal/{t=$2} /MemAvailable/{a=$2} END{print "mem_used_kib=" t-a}' /proc/meminfo
# Userspace processes: kernel threads have an empty cmdline (/proc files report size 0,
# so read them instead of testing -s).
pids=$(for d in /proc/[0-9]*; do [ -n "$(tr -d '\0' < $d/cmdline 2>/dev/null)" ] && echo ${d#/proc/}; done)
echo "processes=$(echo "$pids" | grep -c .)"
p1=$(awk '/VmRSS/{print $2}' /proc/1/status); echo "pid1_rss_kib=$p1"
pss=0; for p in $pids; do v=$(awk '/^Pss:/{print $2}' /proc/$p/smaps_rollup 2>/dev/null); pss=$((pss+${v:-0})); done; echo "userspace_pss_kib=$pss"
"""


class Agent:
    def __init__(self, path):
        self.path = path

    def call(self, cmd, timeout=10):
        s = socket.socket(socket.AF_UNIX)
        s.settimeout(timeout)
        s.connect(self.path)
        f = s.makefile("rb")
        # guest-sync first: replies to requests sent before the agent was up (or by an
        # earlier connection) may still be queued; skip everything up to our token.
        token = int(time.time() * 1000) % 2**31
        s.sendall((json.dumps({"execute": "guest-sync", "arguments": {"id": token}}) + "\n").encode())
        while True:
            line = f.readline()
            if not line:
                raise OSError("agent closed the connection")
            try:
                if json.loads(line).get("return") == token:
                    break
            except ValueError:
                continue
        s.sendall((json.dumps(cmd) + "\n").encode())
        r = json.loads(f.readline())
        s.close()
        if "error" in r:
            raise RuntimeError(r["error"])
        return r["return"]

    def sh(self, script, timeout=30):
        pid = self.call({"execute": "guest-exec", "arguments": {
            "path": "/run/current-system/sw/bin/bash", "arg": ["-lc", script], "capture-output": True}})["pid"]
        end = time.time() + timeout
        while time.time() < end:
            st = self.call({"execute": "guest-exec-status", "arguments": {"pid": pid}})
            if st.get("exited"):
                return base64.b64decode(st.get("out-data", "")).decode()
            time.sleep(0.1)
        raise TimeoutError(script)


def boot(vm, disk, tmp):
    # A fresh directory per boot: the VM runner also starts virtiofsd, whose sockets
    # live in the working directory.
    tmp = tempfile.mkdtemp(dir=tmp)
    qga = os.path.join(tmp, "qga.sock")
    qmp = os.path.join(tmp, "qmp.sock")
    for p in (qga, qmp):
        if os.path.exists(p):
            os.remove(p)
    env = dict(os.environ, NIX_DISK_IMAGE=disk, QEMU_OPTS=(
        f"-display none -serial null -qmp unix:{qmp},server,nowait "
        f"-chardev socket,path={qga},server=on,wait=off,id=qga0 "
        "-device virtio-serial -device virtserialport,chardev=qga0,name=org.qemu.guest_agent.0"))
    runner = [os.path.join(vm, "bin", f) for f in os.listdir(os.path.join(vm, "bin")) if f.startswith("run-")][0]
    proc = subprocess.Popen([runner], env=env, cwd=tmp, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            start_new_session=True)
    agent = Agent(qga)
    deadline = time.time() + 180
    vals = {}
    while time.time() < deadline:
        try:
            out = agent.sh(PROBE)
            if os.environ.get("BENCH_DEBUG"):
                print(f"  probe: {out!r}", flush=True)
            vals = dict(l.split("=", 1) for l in out.split() if "=" in l)
            if vals.get("greeter"):
                break
        except (OSError, RuntimeError, TimeoutError, ValueError) as e:
            if os.environ.get("BENCH_DEBUG"):
                print(f"  probe: {type(e).__name__}: {e}", flush=True)
        time.sleep(0.5)
    if not vals.get("greeter"):
        stop(proc)
        raise RuntimeError(f"{vm}: greeter did not start")
    time.sleep(max(0, float(vals["greeter"]) + 20 - float(agent.sh("cut -d' ' -f1 /proc/uptime"))))
    out = agent.sh(SETTLED)
    vals.update(dict(l.split("=", 1) for l in out.split() if "=" in l))
    stop(proc)
    return vals


def stop(proc):
    """Kill the VM and its helpers (virtiofsd) together."""
    import signal
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    proc.wait()


def main():
    args = sys.argv[1:]
    boots = 5
    if "--boots" in args:
        i = args.index("--boots")
        boots = int(args[i + 1])
        del args[i:i + 2]
    out, variants = args[0], dict(a.split("=", 1) for a in args[1:])
    results = {name: [] for name in variants}
    with tempfile.TemporaryDirectory(prefix="raze-bench-") as tmp:
        disks = {name: os.path.join(tmp, f"{name}.qcow2") for name in variants}
        for name, vm in variants.items():
            print(f"warm-up {name}", flush=True)
            boot(vm, disks[name], tmp)
        for i in range(boots):
            for name, vm in variants.items():
                r = boot(vm, disks[name], tmp)
                print(f"{name} #{i + 1}: {r}", flush=True)
                results[name].append(r)

    summary = {}
    for name, runs in results.items():
        s = {"pid1": runs[0]["pid1"], "boots": len(runs)}
        for key in ("greeter", "network", "mem_used_kib", "processes", "pid1_rss_kib", "userspace_pss_kib"):
            v = [float(r[key]) for r in runs if r.get(key)]
            if v:
                s[key] = {"median": statistics.median(v), "stdev": statistics.stdev(v) if len(v) > 1 else 0.0}
        summary[name] = s
    with open(out, "w") as f:
        json.dump({"summary": summary, "runs": results}, f, indent=2)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
