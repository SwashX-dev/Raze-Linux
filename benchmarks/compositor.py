#!/usr/bin/env python3
"""Compositor benchmark: Rift vs niri on the same Raze system (r6, virgl GPU).

usage: compositor.py OUT.json NAME=VM_RESULT [NAME=VM_RESULT ...] [--runs N]

Each VM_RESULT is benchmarks/variants/desktop.nix built for one desktop (autologin).
QEMU runs a virtio-vga-gl (virgl) GPU on egl-headless, so both compositors render with
GLES on the same virtual GPU. Per run, after the session is up and one foot window is
open and things have settled for 20 s:

  idle      compositor CPU time over 60 s, compositor RSS/PSS
  session   PSS of every process of the session user; MemTotal - MemAvailable
  workload  compositor CPU time while a terminal updates ~20 times/s for 10 s

The compositor process is `rift` or `niri`; the session's shell is Rift Shell
(quickshell) or waybar + mako + swaybg respectively.
"""
import json
import os
import statistics
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from importlib.machinery import SourceFileLoader

di = SourceFileLoader("desktop_init", os.path.join(os.path.dirname(os.path.abspath(__file__)), "desktop-init.py")).load_module()

ENV = ("export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 "
       "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; "
       "export RIFT_SOCKET=$(ls /run/user/1000/rift.*.sock 2>/dev/null); "
       "export NIRI_SOCKET=$(ls /run/user/1000/niri.*.sock 2>/dev/null); ")


def as_user(cmd):
    return "su raze -s /bin/sh -c " + "'" + ENV + cmd.replace("'", "'\\''") + "'"


MEASURE = r"""
tck=$(getconf CLK_TCK)
p=$(pgrep -o -u raze -x COMP)
cpu() { awk '{print $14 + $15}' /proc/$1/stat; }
a=$(cpu $p); sleep SECS; b=$(cpu $p)
echo "cpu_s=$(echo "$a $b $tck" | awk '{printf "%.3f", ($2 - $1) / $3}')"
echo "rss_kib=$(awk '/VmRSS/{print $2}' /proc/$p/status)"
echo "pss_kib=$(awk '/^Pss:/{print $2}' /proc/$p/smaps_rollup)"
s=0; for d in $(pgrep -u raze); do v=$(awk '/^Pss:/{print $2}' /proc/$d/smaps_rollup 2>/dev/null); s=$((s + ${v:-0})); done
echo "session_pss_kib=$s"
awk '/MemTotal/{t=$2} /MemAvailable/{a=$2} END{print "mem_used_kib=" t-a}' /proc/meminfo
"""


def run(vm, disk, tmp):
    tmp = tempfile.mkdtemp(dir=tmp)
    qga = os.path.join(tmp, "qga.sock")
    qmp = os.path.join(tmp, "qmp.sock")
    for p in (qga, qmp):
        if os.path.exists(p):
            os.remove(p)
    env = dict(os.environ, NIX_DISK_IMAGE=disk, QEMU_OPTS=(
        "-vga none -device virtio-vga-gl -display egl-headless "
        f"-serial null -qmp unix:{qmp},server,nowait "
        f"-chardev socket,path={qga},server=on,wait=off,id=qga0 "
        "-device virtio-serial -device virtserialport,chardev=qga0,name=org.qemu.guest_agent.0"))
    runner = [os.path.join(vm, "bin", f) for f in os.listdir(os.path.join(vm, "bin")) if f.startswith("run-")][0]
    proc = subprocess.Popen([runner], env=env, cwd=tmp, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            start_new_session=True)
    agent = di.Agent(qga)
    try:
        comp = None
        deadline = time.time() + 240
        while time.time() < deadline and comp is None:
            try:
                out = agent.sh("pgrep -u raze -x rift >/dev/null && echo rift; pgrep -u raze -x niri >/dev/null && echo niri")
                comp = out.split()[0] if out.split() else None
            except (OSError, RuntimeError, TimeoutError):
                pass
            time.sleep(1)
        if comp is None:
            raise RuntimeError(f"{vm}: no compositor")
        time.sleep(10)
        spawn = "rift msg action spawn foot" if comp == "rift" else "niri msg action spawn -- foot"
        agent.sh(as_user(spawn))
        renderer = agent.sh(as_user("rift msg version") if comp == "rift" else "true").strip()
        time.sleep(20)
        idle = dict(l.split("=", 1) for l in agent.sh(MEASURE.replace("COMP", comp).replace("SECS", "60"), timeout=90).split())
        load = "for i in $(seq 1 200); do date +%T.%N; sleep 0.05; done"
        spawn = (f"rift msg action spawn \"foot sh -c '{load}'\"" if comp == "rift"
                 else f"niri msg action spawn -- foot sh -c \"{load}\"")
        agent.sh(as_user(spawn))
        time.sleep(1)
        work = dict(l.split("=", 1) for l in agent.sh(MEASURE.replace("COMP", comp).replace("SECS", "10"), timeout=40).split())
        return {"compositor": comp, "renderer": renderer,
                "idle_cpu_s_per_60s": float(idle["cpu_s"]), "rss_kib": int(idle["rss_kib"]),
                "pss_kib": int(idle["pss_kib"]), "session_pss_kib": int(idle["session_pss_kib"]),
                "mem_used_kib": int(idle["mem_used_kib"]), "workload_cpu_s_per_10s": float(work["cpu_s"])}
    finally:
        di.stop(proc)


def main():
    args = sys.argv[1:]
    runs = 3
    if "--runs" in args:
        i = args.index("--runs")
        runs = int(args[i + 1])
        del args[i:i + 2]
    out, variants = args[0], dict(a.split("=", 1) for a in args[1:])
    results = {n: [] for n in variants}
    with tempfile.TemporaryDirectory(prefix="raze-bench-") as tmp:
        disks = {n: os.path.join(tmp, f"{n}.qcow2") for n in variants}
        for n, vm in variants.items():
            print(f"warm-up {n}: {run(vm, disks[n], tmp)}", flush=True)
        for i in range(runs):
            for n, vm in variants.items():
                r = run(vm, disks[n], tmp)
                print(f"{n} #{i + 1}: {r}", flush=True)
                results[n].append(r)
    summary = {}
    for n, rs in results.items():
        s = {"compositor": rs[0]["compositor"], "renderer": rs[0]["renderer"], "runs": len(rs)}
        for k in ("idle_cpu_s_per_60s", "rss_kib", "pss_kib", "session_pss_kib", "mem_used_kib", "workload_cpu_s_per_10s"):
            v = [r[k] for r in rs]
            s[k] = {"median": statistics.median(v), "stdev": statistics.stdev(v) if len(v) > 1 else 0.0}
        summary[n] = s
    with open(out, "w") as f:
        json.dump({"summary": summary, "runs": results}, f, indent=2)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
