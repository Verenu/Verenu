"""Disposable GTK entry for the opt-in AT-SPI formatting test."""

import os

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk

window = Gtk.Window(title="Verenu formatting verification")
entry = Gtk.Entry()
window.add(entry)
window.connect("destroy", Gtk.main_quit)
window.show_all()
entry.grab_focus()
GLib.timeout_add_seconds(300, lambda: window.destroy())
print(f"[INFO] Fixture PID: {os.getpid()}; closes after five minutes", flush=True)
Gtk.main()
