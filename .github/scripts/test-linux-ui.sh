#!/usr/bin/env bash
set -euo pipefail
appimage="$(realpath "$1")"
evidence="$(realpath -m "$2")"
mkdir -p "$evidence"
test_root="$(mktemp -d)"
export XDG_RUNTIME_DIR="$test_root/runtime"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
weston_pid=""
app_pid=""
cleanup() {
  if [[ -n "$app_pid" ]]; then kill "$app_pid" 2>/dev/null || true; fi
  if [[ -n "$weston_pid" ]]; then kill "$weston_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT
weston --backend=x11-backend.so --use-pixman --socket=wayland-mouzi --xwayland --width=1280 --height=900 --idle-time=0 >"$evidence/weston.log" 2>&1 &
weston_pid=$!
for attempt in $(seq 1 100); do
  [[ -S "$XDG_RUNTIME_DIR/wayland-mouzi" ]] && break
  sleep .2
done
test -S "$XDG_RUNTIME_DIR/wayland-mouzi"
for backend in wayland x11; do
  export XDG_CONFIG_HOME="$test_root/$backend/config"
  export XDG_DATA_HOME="$test_root/$backend/data"
  mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME/mouzi" "$test_root/files"
  python3 - <<'PY'
import os, sqlite3
db = sqlite3.connect(os.path.join(os.environ['XDG_DATA_HOME'], 'mouzi', 'mouzi.db'))
db.execute('CREATE TABLE settings(id INTEGER PRIMARY KEY, language TEXT, theme TEXT, telemetry_enabled INTEGER, first_run INTEGER, autostart INTEGER)')
db.execute("INSERT INTO settings VALUES(1,'en','light',0,0,0)")
db.commit()
PY
  app_display="$DISPLAY"
  if [[ "$backend" == x11 ]]; then
    app_display="$(sed -n 's/.*xserver listening on display \(:[0-9]*\).*/\1/p' "$evidence/weston.log" | tail -1)"
    test -n "$app_display"
  fi
  env DISPLAY="$app_display" WAYLAND_DISPLAY=wayland-mouzi GDK_BACKEND="$backend" LIBGL_ALWAYS_SOFTWARE=1 "$appimage" --add-folder "$test_root/files" >"$evidence/$backend.log" 2>&1 &
  app_pid=$!
  passed=false
  for attempt in $(seq 1 30); do
    sleep 1
    kill -0 "$app_pid"
    import -window root "$evidence/$backend.png"
    tesseract "$evidence/$backend.png" "$evidence/$backend" 2>/dev/null
    if grep -Eiq 'Watched Folders|Review queue' "$evidence/$backend.txt"; then passed=true; break; fi
  done
  test "$passed" = true
  echo "PASS Settings rendered under $backend (software rendering in Weston)"
  kill "$app_pid"
  wait "$app_pid" || true
  app_pid=""
done
