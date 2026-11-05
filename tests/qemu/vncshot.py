#!/usr/bin/env python3
"""Minimal RFB client: grab one full frame from a QEMU VNC unix socket as PPM.

QEMU cannot `screendump` a GL (virgl/egl-headless) display, but its VNC server keeps
a copy of the scanout. usage: vncshot.py SOCK OUT.ppm"""
import socket
import struct
import sys


def recv(s, n):
    b = b""
    while len(b) < n:
        c = s.recv(n - len(b))
        if not c:
            raise EOFError
        b += c
    return b


s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
recv(s, 12)
s.sendall(b"RFB 003.008\n")
n = recv(s, 1)[0]
types = recv(s, n)
if 1 not in types:
    sys.exit("server requires authentication")
s.sendall(b"\x01")
if struct.unpack(">I", recv(s, 4))[0] != 0:
    sys.exit("security handshake failed")
s.sendall(b"\x01")  # shared
w, h = struct.unpack(">HH", recv(s, 4))
recv(s, 16)
recv(s, struct.unpack(">I", recv(s, 4))[0])
# 32bpp, little endian, true colour, r/g/b max 255, shifts 16/8/0
s.sendall(struct.pack(">BxxxBBBBHHHBBBxxx", 0, 32, 24, 0, 1, 255, 255, 255, 16, 8, 0))
s.sendall(struct.pack(">BxH", 2, 1) + struct.pack(">i", 0))  # Raw encoding only
s.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, w, h))
fb = bytearray(w * h * 3)
got = 0
while got < w * h:
    t = recv(s, 1)[0]
    if t != 0:
        sys.exit(f"unexpected message {t}")
    recv(s, 1)
    nrects = struct.unpack(">H", recv(s, 2))[0]
    for _ in range(nrects):
        x, y, rw, rh, enc = struct.unpack(">HHHHi", recv(s, 12))
        if enc != 0:
            sys.exit(f"unexpected encoding {enc}")
        data = recv(s, rw * rh * 4)
        for row in range(rh):
            for col in range(rw):
                i = (row * rw + col) * 4
                o = ((y + row) * w + x + col) * 3
                fb[o:o + 3] = bytes((data[i + 2], data[i + 1], data[i]))
        got += rw * rh
with open(sys.argv[2], "wb") as f:
    f.write(b"P6\n%d %d\n255\n" % (w, h) + fb)
