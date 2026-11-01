#!/usr/bin/env bash
# Run in a disposable X11, D-Bus, network and XDG session. The app path is replaced.
# Arguments: fresh test directory, disposable AppImage inside it, success|fail, update AppImage.
set -euo pipefail
umask 077
root="$(realpath -m "${1:?test directory required}")"
binary="$(realpath -m "${2:?disposable app required}")"
mode="${3:?success or fail required}"
[[ "${VERENU_UPDATE_TEST_ISOLATED:-}" = 1 && "${XDG_DATA_HOME:-}" = "$root/share" && "${XDG_RUNTIME_DIR:-}" = "$root/runtime" ]] || { echo "Use a private X11, D-Bus, network and XDG session." >&2; exit 2; }
[[ "$binary" = "$root/"* && ! -e "$root/data/settings.json" ]] || { echo "Use a fresh directory and a disposable app inside it." >&2; exit 2; }
[[ "$mode" = success || "$mode" = fail ]] || exit 2
update_image="${4:-}"
mkdir -p "$root"/{data,config,state,runtime}
chmod 700 "$root/runtime"
export XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config" XDG_STATE_HOME="$root/state"
export VERENU_APP_DATA_DIR_OVERRIDE="$root/data" VERENU_HANDOFF_TEST_TRIGGER="$root/trigger"
export VERENU_HANDOFF_TEST_SHORTCUTS=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 NO_AT_BRIDGE=1 GSETTINGS_BACKEND=memory
if [[ "$mode" = fail ]]; then export VERENU_HANDOFF_TEST_FAIL_AFTER_COMMIT=1; fi
printf synthetic-test-password | gnome-keyring-daemon --unlock >/dev/null
printf synthetic-key-value | secret-tool store --label=verenu-test service com.verenu.app username api_key_groq
server=''
if [[ -n "$update_image" ]]; then
  original_image_hash=$(sha256sum "$binary" | cut -d ' ' -f 1)
  export VERENU_UPDATE_TEST_LOCAL_URL="http://127.0.0.1:18765/$(basename "$update_image")"
  export VERENU_UPDATE_TEST_SHA256
  VERENU_UPDATE_TEST_SHA256=$(sha256sum "$update_image" | cut -d ' ' -f 1)
  /usr/bin/python3 -m http.server 18765 --bind 127.0.0.1 --directory "$(dirname "$update_image")" > "$root/server.log" 2>&1 &
  server=$!
fi
printf '%s\n' '{"setup_complete":true,"analytics_enabled":false,"cleanup_cache_enabled":true}' > "$root/data/settings.json"
cat > "$root/watcher.py" <<'PY_WATCHER'
from gi.repository import Gio, GLib

xml = '''<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
</interface></node>'''
node = Gio.DBusNodeInfo.new_for_xml(xml)
loop = GLib.MainLoop()

def acquired(connection, name):
    def method(conn, sender, path, interface, method_name, params, invocation):
        invocation.return_value(None)
    def property_value(conn, sender, path, interface, name):
        return GLib.Variant('b', True)
    connection.register_object('/StatusNotifierWatcher', node.interfaces[0], method, property_value, None)

Gio.bus_own_name(Gio.BusType.SESSION, 'org.kde.StatusNotifierWatcher', Gio.BusNameOwnerFlags.NONE, acquired, None, None)
loop.run()
PY_WATCHER
/usr/bin/python3 "$root/watcher.py" > "$root/watcher.log" 2>&1 &
watcher=$!
"$binary" > "$root/app.log" 2>&1 &
parent=$!
child=''
cleanup() {
  if [[ -n "$child" ]]; then kill "$child" 2>/dev/null || true; fi
  kill "$parent" 2>/dev/null || true
  kill "$watcher" 2>/dev/null || true
  if [[ -n "$server" ]]; then kill "$server" 2>/dev/null || true; fi
  wait "$parent" 2>/dev/null || true
}
trap cleanup EXIT
for _ in $(seq 1 300); do
  if gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetNameOwner com.verenu.app.SingleInstance > "$root/owner-before" 2>/dev/null; then break; fi
  if ! kill -0 "$parent" 2>/dev/null; then cat "$root/app.log"; exit 1; fi
  sleep .1
done
sleep 8
scrot "$root/before.png" || true
/usr/bin/python3 - "$root/data/verenu.db" <<'PY'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    db.execute('CREATE TABLE IF NOT EXISTS handoff_fixture (value TEXT NOT NULL)')
    db.execute("INSERT INTO handoff_fixture VALUES ('synthetic')")
PY
sha256sum "$root/data/sync-identity.key" > "$root/identity.before"
touch "$root/trigger"
if [[ "$mode" = fail ]]; then
  for _ in $(seq 1 450); do
    if [[ -e "$root/trigger.committing" || -e "$root/trigger.error" ]]; then break; fi
    sleep .1
  done
  test -s "$root/trigger.committing"
  window=$(xdotool search --onlyvisible --name Verenu | head -1)
  xdotool windowunmap "$window"
  gdbus call --session --dest com.verenu.app.SingleInstance --object-path /com/verenu/app/SingleInstance --method org.SingleInstance.DBus.ExecuteCallback '[]' ''
  touch "$root/trigger.continue"
fi
for _ in $(seq 1 900); do
  if [[ -e "$root/trigger.error" || -e "$root/trigger.activated" ]]; then break; fi
  sleep .1
done
if [[ "$mode" = fail ]]; then
  test -s "$root/trigger.error"
  grep -q 'Synthetic late activation failure' "$root/trigger.error"
  cat "$root/trigger.error"
  kill -0 "$parent"
  owner_after=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetNameOwner com.verenu.app.SingleInstance)
  test "$owner_after" = "$(cat "$root/owner-before")"
  gdbus call --session --dest com.verenu.app.SingleInstance --object-path /com/verenu/app/SingleInstance --method org.SingleInstance.DBus.ExecuteCallback "['--verenu-hotkey-release']" ''
  for _ in $(seq 1 50); do
    if xdotool search --onlyvisible --name Verenu 2>/dev/null | grep -qx "$window"; then break; fi
    sleep .1
  done
  xdotool search --onlyvisible --name Verenu | grep -qx "$window"
else
  if [[ -e "$root/trigger.error" ]]; then cat "$root/trigger.error"; exit 1; fi
  test -s "$root/trigger.activated"
  child=$(cat "$root/trigger.activated")
  kill -0 "$child"
  if kill -0 "$parent" 2>/dev/null; then echo 'parent still running'; exit 1; fi
  owner=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetNameOwner com.verenu.app.SingleInstance | cut -d "'" -f 2)
  owner_pid=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetConnectionUnixProcessID "$owner" | sed -E 's/.*uint32 ([0-9]+).*/\1/')
  test "$owner_pid" = "$child"
  gdbus call --session --dest com.verenu.app.SingleInstance --object-path /com/verenu/app/SingleInstance --method org.SingleInstance.DBus.ExecuteCallback "['--verenu-hotkey-release']" ''
  sleep 8
  kill -0 "$child"
  window=$(xdotool search --onlyvisible --name Verenu | head -1)
  xdotool windowunmap "$window"
  gdbus call --session --dest com.verenu.app.SingleInstance --object-path /com/verenu/app/SingleInstance --method org.SingleInstance.DBus.ExecuteCallback '[]' ''
  for _ in $(seq 1 50); do
    if xdotool search --onlyvisible --name Verenu 2>/dev/null | grep -qx "$window"; then break; fi
    sleep .1
  done
  xdotool search --onlyvisible --name Verenu | grep -qx "$window"
  scrot "$root/after.png" || true
  echo "replacement=$child"
fi
sha256sum -c "$root/identity.before"
test "$(secret-tool lookup service com.verenu.app username api_key_groq)" = synthetic-key-value
if [[ -n "$update_image" ]]; then
  test -s "$binary.previous"
  test "$(sha256sum "$binary.previous" | cut -d ' ' -f 1)" = "$original_image_hash"
  test "$(sha256sum "$binary" | cut -d ' ' -f 1)" = "$VERENU_UPDATE_TEST_SHA256"
  test -s "$root/data/verenu.db.bak"
  /usr/bin/python3 - "$root/data/verenu.db.bak" <<'PY'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    assert db.execute('SELECT value FROM handoff_fixture').fetchone() == ('synthetic',)
PY
fi
/usr/bin/python3 - "$root/data/verenu.db" "$root/data/settings.json" <<'PY'
import json, sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    assert db.execute('SELECT value FROM handoff_fixture').fetchone() == ('synthetic',)
with open(sys.argv[2]) as settings:
    assert json.load(settings)['setup_complete'] is True
PY
