"""Disposable GTK entry for the opt-in AT-SPI formatting test."""

import argparse
import json
import os
import sys

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--auto-learn", action="store_true", help="Accept synthetic text commands on stdin")
args = parser.parse_args()
window = Gtk.Window(title="Verenu formatting verification")
entry = Gtk.Entry()
if args.auto_learn:
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    alternate = Gtk.Entry()
    secure = Gtk.Entry()
    secure.set_visibility(False)
    for field in [entry, alternate, secure]:
        box.pack_start(field, False, False, 0)
    window.add(box)

    def command(source, condition):
        if condition & GLib.IO_HUP:
            window.destroy()
            return False
        try:
            line = sys.stdin.readline()
            if not line:
                window.destroy()
                return False
            payload = json.loads(line)
            field = [entry, alternate, secure][payload["field"]]
            field.set_text(payload["text"])
            field.set_position(-1)
            field.grab_focus()
            print("[SUCCESS] Synthetic edit applied", flush=True)
        except (ValueError, KeyError, IndexError) as error:
            print(f"[ERROR] Invalid fixture command: {type(error).__name__}", flush=True)
        return True

    GLib.io_add_watch(sys.stdin, GLib.IO_IN | GLib.IO_HUP, command)
else:
    window.add(entry)
window.connect("destroy", Gtk.main_quit)
window.show_all()
entry.grab_focus()
GLib.timeout_add_seconds(300, lambda: window.destroy())
print(f"[INFO] Fixture PID: {os.getpid()}; closes after five minutes", flush=True)
Gtk.main()
