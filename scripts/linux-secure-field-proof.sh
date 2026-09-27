#!/usr/bin/env bash
# Kaydence — live secure-field proof on Hyprland (ADR-0023, non-negotiable #8).
#
# Drives the SHIPPED Linux delivery path (LinuxTextInjector: AT-SPI focus
# tracker + Hyprland pid match → inject_committed_text → virtual keyboard) at a
# real GTK4 window, three times:
#   1. a password entry  → must be REFUSED (Held{SecureField}), entry stays empty
#   2. a password entry that already had focus BEFORE the tracker started (no
#      focus event to see) → must still be REFUSED via the on-demand lookup
#   3. a normal entry    → must be TYPED, entry holds exactly the text
#
#   bash scripts/linux-secure-field-proof.sh [--help]
#
# Needs: Hyprland, python3 + PyGObject with GTK 4, the AT-SPI bus
# (at-spi2-core). Opens three small throwaway windows, moves focus to each and
# gives it back; writes only to a mktemp dir. Exit 0 = all three proven.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
  sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi
fail() { echo "[secure] FAIL: $*" >&2; exit 1; }
[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] || fail "not a Hyprland session"
python3 -c 'import gi; gi.require_version("Gtk", "4.0")' 2>/dev/null || fail "PyGObject with GTK 4 is required"
# A locked session (Omarchy/Quickshell ext-session-lock) keeps keyboard focus on
# the lock screen: refuse up front with a clear reason instead of a focus miss.
if hyprctl -j monitors | jq -e 'any(.[]; (.solitaryBlockedBy // []) | index("LOCK"))' >/dev/null 2>&1; then
  fail "the session is locked — unlock it and rerun (the lock screen holds keyboard focus)"
fi

cargo build --quiet --manifest-path apps/desktop/src-tauri/Cargo.toml --bin wayland-selftest
BIN=target/debug/wayland-selftest

WORK="$(mktemp -d)"
APP_PID=""
cleanup() {
  [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# A one-entry GTK4 window. On SIGTERM it writes the entry's current text to
# the output file, so we can prove what did (or did not) land in the field.
cat > "$WORK/field.py" <<'PY'
import signal, sys, gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gtk, GLib
mode, app_id, out = sys.argv[1], sys.argv[2], sys.argv[3]
app = Gtk.Application(application_id=app_id)
def activate(a):
    win = Gtk.ApplicationWindow(application=a, title="Kaydence secure-field proof")
    entry = Gtk.PasswordEntry() if mode == "password" else Gtk.Entry()
    win.set_child(entry)
    win.present()
    entry.grab_focus()
    def dump(*_):
        with open(out, "w") as f:
            f.write(entry.get_text())
        a.quit()
        return GLib.SOURCE_REMOVE
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, dump)
app.connect("activate", activate)
app.run(None)
PY

HOME_WIN="$(hyprctl activewindow -j | jq -r '.address // empty')"
GOT=""
run_case() { # mode app_id text expected_exit [prefocused] → sets GOT (runs in this shell so the trap cleans up)
  local mode="$1" app_id="$2" text="$3" want="$4" prefocused="${5:-}" out="$WORK/$2.txt" addr="" rc=0
  python3 "$WORK/field.py" "$mode" "$app_id" "$out" &
  APP_PID=$!
  for _ in $(seq 1 100); do
    addr="$(hyprctl clients -j | jq -r --arg c "$app_id" '.[] | select(.class == $c) | .address' | head -n1)"
    [ -n "$addr" ] && break
    sleep 0.05
  done
  [ -n "$addr" ] || fail "$mode: proof window never mapped"
  # A new window takes focus the instant it maps — before the selftest's AT-SPI
  # tracker exists. The running app's tracker is up long before any field gets
  # focus, so mirror that: hand focus back first; the selftest then starts its
  # tracker and moves focus to the field itself.
  if [ -n "$HOME_WIN" ] && [ -z "$prefocused" ]; then
    hyprctl dispatch "hl.dsp.focus({ window = \"address:$HOME_WIN\" })" >/dev/null
    for _ in $(seq 1 40); do
      [ "$(hyprctl activewindow -j | jq -r '.address')" = "$HOME_WIN" ] && break; sleep 0.05
    done
  fi
  "$BIN" --guarded-type-into "$addr" "$text" >&2 || rc=$?
  sleep 0.3
  kill -TERM "$APP_PID"; wait "$APP_PID" 2>/dev/null || true; APP_PID=""
  GOT="$(cat "$out" 2>/dev/null || true)"
  [ "$rc" = "$want" ] || fail "$mode: selftest exit $rc, expected $want"
}

run_case password io.kaydence.SecureFieldProof 'kaydence-must-not-type-this' 10
[ -z "$GOT" ] || fail "password entry received text (${#GOT} chars) — refusal did not hold"
echo "[secure] PASS password entry: Held{SecureField}, field empty"

run_case password io.kaydence.SecureFieldPrefocused 'kaydence-must-not-type-this' 10 prefocused
[ -z "$GOT" ] || fail "pre-focused password entry received text (${#GOT} chars) — on-demand lookup did not hold"
echo "[secure] PASS pre-focused password entry (no focus event): Held{SecureField}, field empty"

run_case text io.kaydence.TextFieldProof 'Kaydence typed this into GTK' 0
[ "$GOT" = "Kaydence typed this into GTK" ] || fail "text entry holds '$GOT'"
echo "[secure] PASS text entry: Injected, field holds the exact text"
echo "[secure] RESULT: PASS (secure fields refused incl. focused-before-tracking, normal field typed — shipped path)"
