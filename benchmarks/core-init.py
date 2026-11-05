#!/usr/bin/env python3
"""r6 vs s6 boot benchmark on the Raze Core image.

Boots the same kernel + initramfs under QEMU/KVM with either r6-init or an
equivalent s6-svscan PID 1, supervising the same services (20 benchmark services
that record CLOCK_BOOTTIME when they start, plus network, console and a logged
ticker). Parses the self-test report from the serial console.

Metrics per boot:
  pid1_to_services_ms   last benchmark service started - PID 1 start (CLOCK_BOOTTIME)
  services_started_ms   last benchmark service started, since kernel start
  supervision_rss_kib   summed VmRSS of init + supervision (+ logger) processes
  supervision_pss_kib   summed Pss (shared pages divided among sharers)
  supervision_procs     number of those processes
  mem_used_kib          MemTotal - MemAvailable after boot

Usage: benchmarks/core-init.py IMAGE_DIR [--runs N] [--json OUT]
"""

import argparse
import json
import re
import statistics
import subprocess
import sys
import time

RESULT = re.compile(
    r"RAZE-CORE: RESULT init=(\S+) services_started_ms=([\d.]+) pid1_start_ms=([\d.]+) "
    r"supervision_processes=(\d+) supervision_rss_kib=(\d+) supervision_pss_kib=(\d+) "
    r"mem_used_kib=(\d+) processes=(\d+)"
)


def boot(image, init, timeout=60):
    cmd = [
        "qemu-system-x86_64", "-enable-kvm", "-cpu", "host", "-m", "256", "-smp", "2",
        "-nographic", "-no-reboot", "-kernel", f"{image}/bzImage", "-initrd", f"{image}/initrd",
        "-append", f"console=ttyS0 rdinit=/sbin/{init} raze.autotest quiet panic=-1",
        "-nic", "user,model=virtio-net-pci",
    ]
    t0 = time.monotonic()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
    out = []
    try:
        while time.monotonic() - t0 < timeout:
            line = p.stdout.readline()
            if not line:
                break
            line = line.decode("utf-8", "replace")
            out.append(line)
            if "RAZE-CORE: selftest done" in line:
                break
    finally:
        p.kill()
        p.wait()
    text = "".join(out)
    m = RESULT.search(text)
    if not m:
        raise RuntimeError(f"incomplete report for {init}:\n{text[-2000:]}")
    r = {"init": init, "wall_to_selftest_s": round(time.monotonic() - t0, 3)}
    r["services_started_ms"] = float(m.group(2))
    r["pid1_start_ms"] = float(m.group(3))
    r["pid1_to_services_ms"] = round(r["services_started_ms"] - r["pid1_start_ms"], 3)
    r["supervision_procs"] = int(m.group(4))
    r["supervision_rss_kib"] = int(m.group(5))
    r["supervision_pss_kib"] = int(m.group(6))
    r["mem_used_kib"] = int(m.group(7))
    r["processes"] = int(m.group(8))
    return r


def summarize(rows, key):
    vals = [r[key] for r in rows]
    return {
        "median": round(statistics.median(vals), 2),
        "mean": round(statistics.mean(vals), 2),
        "stdev": round(statistics.stdev(vals), 2) if len(vals) > 1 else 0.0,
        "min": min(vals),
        "max": max(vals),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("image")
    ap.add_argument("--runs", type=int, default=10)
    ap.add_argument("--json")
    a = ap.parse_args()
    results = {"r6-init": [], "s6-init": []}
    retries = {"r6-init": 0, "s6-init": 0}
    # Interleave runs so drift on the host affects both equally.
    for i in range(a.runs):
        for init in results:
            for attempt in range(3):
                try:
                    r = boot(a.image, init)
                    break
                except RuntimeError as e:
                    retries[init] += 1
                    print(f"run {i + 1:2d} {init}: incomplete serial report, retrying ({e.args[0].splitlines()[0]})", flush=True)
            else:
                raise RuntimeError(f"{init}: three incomplete boots in a row")
            results[init].append(r)
            print(f"run {i + 1:2d} {init:8s} pid1->services {r['pid1_to_services_ms']:8.2f} ms  "
                  f"supervision {r['supervision_procs']:2d} procs rss {r['supervision_rss_kib']:6d} pss {r['supervision_pss_kib']:6d} KiB  "
                  f"mem {r['mem_used_kib']} KiB", flush=True)
    summary = {}
    for init, rows in results.items():
        summary[init] = {k: summarize(rows, k) for k in
                         ["pid1_to_services_ms", "services_started_ms", "supervision_rss_kib", "supervision_pss_kib",
                          "supervision_procs", "mem_used_kib"]}
    summary["retries"] = retries
    print(json.dumps(summary, indent=2))
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"runs": results, "summary": summary}, f, indent=2)


if __name__ == "__main__":
    sys.exit(main())
