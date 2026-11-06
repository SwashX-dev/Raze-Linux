# Start a screen cast through the XDG desktop portal (as a browser would) and print
# the PipeWire node of the stream. Exits non-zero if any step is refused.
import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

bus = Gio.bus_get_sync(Gio.BusType.SESSION)
sender = bus.get_unique_name()[1:].replace(".", "_")
loop = GLib.MainLoop()


def request(method, fmt, args, token):
    path = f"/org/freedesktop/portal/desktop/request/{sender}/{token}"
    result = {}

    def on_response(_conn, _sender, _path, _iface, _signal, params, *_):
        result["r"] = params.unpack()
        loop.quit()

    sid = bus.signal_subscribe(None, "org.freedesktop.portal.Request", "Response",
                               path, None, Gio.DBusSignalFlags.NONE, on_response)
    bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
                  "org.freedesktop.portal.ScreenCast", method, GLib.Variant(fmt, args),
                  None, Gio.DBusCallFlags.NONE, 60000, None)
    timer = GLib.timeout_add_seconds(60, loop.quit)
    loop.run()
    GLib.source_remove(timer)
    bus.signal_unsubscribe(sid)
    if "r" not in result:
        sys.exit(f"{method}: no response")
    code, res = result["r"]
    if code != 0:
        sys.exit(f"{method}: response {code}")
    return res


tok = lambda t: GLib.Variant("s", t)  # noqa: E731
res = request("CreateSession", "(a{sv})",
              ({"handle_token": tok("t1"), "session_handle_token": tok("s1")},), "t1")
session = res["session_handle"]
request("SelectSources", "(oa{sv})",
        (session, {"handle_token": tok("t2"), "types": GLib.Variant("u", 1)}), "t2")
res = request("Start", "(osa{sv})", (session, "", {"handle_token": tok("t3")}), "t3")
node = res["streams"][0][0]
print(node, flush=True)
# Keep the session (and the stream) alive until killed.
GLib.timeout_add_seconds(120, loop.quit)
loop.run()
