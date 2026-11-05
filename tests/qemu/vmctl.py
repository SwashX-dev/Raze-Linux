#!/usr/bin/env python3
"""Drive a QEMU VM over QMP: screenshots, keyboard, absolute mouse.

Used for installer testing: it operates the real graphical installer the way a
user would, and captures real screenshots.

  vmctl.py SOCK screenshot OUT.png
  vmctl.py SOCK key ctrl-alt-t          (QEMU qcode names joined by '-')
  vmctl.py SOCK type 'some text'
  vmctl.py SOCK click X Y [--screen WxH] (coordinates in screenshot pixels)
  vmctl.py SOCK dblclick X Y
  vmctl.py SOCK cmd '{"execute": "query-status"}'
"""

import json
import socket
import subprocess
import sys
import time

QCODE = {
    " ": "spc", "\n": "ret", "\t": "tab", "-": "minus", "=": "equal", "[": "bracket_left",
    "]": "bracket_right", ";": "semicolon", "'": "apostrophe", ",": "comma", ".": "dot",
    "/": "slash", "\\": "backslash", "`": "grave_accent",
}
SHIFTED = {
    "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7", "*": "8", "(": "9",
    ")": "0", "_": "minus", "+": "equal", "{": "bracket_left", "}": "bracket_right",
    ":": "semicolon", '"': "apostrophe", "<": "comma", ">": "dot", "?": "slash", "|": "backslash",
    "~": "grave_accent",
}


class QMP:
    def __init__(self, path):
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.connect(path)
        self.f = self.s.makefile("rw")
        json.loads(self.f.readline())  # greeting
        self.cmd({"execute": "qmp_capabilities"})

    def cmd(self, obj):
        self.f.write(json.dumps(obj) + "\n")
        self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if "return" in r or "error" in r:
                if "error" in r:
                    raise RuntimeError(r["error"])
                return r["return"]

    def keys(self, qcodes, hold=0.05):
        self.cmd({"execute": "send-key", "arguments": {
            "keys": [{"type": "qcode", "data": k} for k in qcodes], "hold-time": int(hold * 1000)}})

    def type(self, text):
        for ch in text:
            if ch.isalpha() and ch.isupper():
                self.keys(["shift", ch.lower()])
            elif ch in SHIFTED:
                self.keys(["shift", SHIFTED[ch]])
            elif ch in QCODE:
                self.keys([QCODE[ch]])
            else:
                self.keys([ch])
            time.sleep(0.03)

    def screen_size(self):
        tmp = "/tmp/.vmctl-size.ppm"
        self.cmd({"execute": "screendump", "arguments": {"filename": tmp}})
        with open(tmp, "rb") as f:
            f.readline()
            line = f.readline()
            while line.startswith(b"#"):
                line = f.readline()
            w, h = map(int, line.split())
        return w, h

    def move(self, x, y):
        w, h = self.screen_size()
        ax, ay = int(x * 32767 / (w - 1)), int(y * 32767 / (h - 1))
        self.cmd({"execute": "input-send-event", "arguments": {"events": [
            {"type": "abs", "data": {"axis": "x", "value": ax}},
            {"type": "abs", "data": {"axis": "y", "value": ay}}]}})

    def click(self, x, y, button="left", count=1):
        self.move(x, y)
        time.sleep(0.1)
        for _ in range(count):
            for down in (True, False):
                self.cmd({"execute": "input-send-event", "arguments": {"events": [
                    {"type": "btn", "data": {"down": down, "button": button}}]}})
                time.sleep(0.06)

    def screenshot(self, out):
        ppm = out + ".ppm"
        self.cmd({"execute": "screendump", "arguments": {"filename": ppm}})
        if out.endswith(".png"):
            # Pillow may be unavailable; ImageMagick is used through nix in the test env.
            try:
                from PIL import Image  # type: ignore
                Image.open(ppm).save(out)
            except ImportError:
                subprocess.run(["magick", ppm, out], check=True)
            import os
            os.unlink(ppm)


def main():
    sock, op, *args = sys.argv[1:]
    q = QMP(sock)
    if op == "screenshot":
        q.screenshot(args[0])
    elif op == "key":
        q.keys(args[0].split("-"))
    elif op == "type":
        q.type(args[0])
    elif op == "click":
        q.click(int(args[0]), int(args[1]))
    elif op == "dblclick":
        q.click(int(args[0]), int(args[1]), count=2)
    elif op == "rclick":
        q.click(int(args[0]), int(args[1]), button="right")
    elif op == "move":
        q.move(int(args[0]), int(args[1]))
    elif op == "cmd":
        print(json.dumps(q.cmd(json.loads(args[0])), indent=2))
    else:
        sys.exit(f"unknown op {op}")


if __name__ == "__main__":
    main()
