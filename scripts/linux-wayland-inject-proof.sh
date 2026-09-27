#!/usr/bin/env bash
# Kaydence — Wayland virtual-keyboard injection proof (ADR-0023, P1-P0-3).
#
# Proves, with a byte-exact readback, that the rootless zwp_virtual_keyboard_v1
# path types full Unicode into a real focused Wayland client on Hyprland
# (Omarchy). Unlike the GNOME uinput demo, no human needs to move focus:
# Hyprland IPC can, and the selftest re-verifies focus before the first key
# (Pitfall P9) and refuses to type if anything else holds it.
#
#   bash scripts/linux-wayland-inject-proof.sh            # default corpus
#   bash scripts/linux-wayland-inject-proof.sh --help
#
# Needs: a Hyprland session, foot, jq (all Omarchy defaults). Writes only to a
# mktemp dir, removed on exit. Exit 0 = every case matched byte-for-byte.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
  sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

fail() { echo "[proof] FAIL: $*" >&2; exit 1; }
[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] || fail "not a Hyprland session"
command -v foot >/dev/null || fail "foot is required (Omarchy default terminal)"
command -v jq >/dev/null || fail "jq is required"
# A locked session (Omarchy/Quickshell ext-session-lock) keeps keyboard focus on
# the lock screen: refuse up front with a clear reason instead of a focus miss.
if hyprctl -j monitors | jq -e 'any(.[]; (.solitaryBlockedBy // []) | index("LOCK"))' >/dev/null 2>&1; then
  fail "the session is locked — unlock it and rerun (the lock screen holds keyboard focus)"
fi

cargo build --quiet --manifest-path apps/desktop/src-tauri/Cargo.toml --bin wayland-selftest
BIN=target/debug/wayland-selftest
"$BIN" --probe

WORK="$(mktemp -d)"
FOOT_PID=""
cleanup() {
  [ -n "$FOOT_PID" ] && kill "$FOOT_PID" 2>/dev/null || true
  for pid in $(hyprctl clients -j 2>/dev/null | jq -r '.[] | select(.class | startswith("kaydence-inject-proof-steal")) | .pid'); do
    kill "$pid" 2>/dev/null || true
  done
  rm -rf "$WORK"
}
trap cleanup EXIT

# name|text — ASCII, Latin-1, symbols, CJK, RTL, combining mark, astral emoji,
# an everyday sentence that needs two keymaps (> 48 distinct chars), and a long
# passage that forces many keymap chunks (each chunk = a fresh virtual keyboard).
LONG="$(python3 -c 'print("".join(chr(c) for c in range(0x4e00, 0x4e00 + 300)))')"
CASES=(
  "ascii|Hello from Kaydence on Omarchy, 100% local!"
  "latin1|Café déjà vu — naïve façade, jalapeño ¿qué?"
  "cjk|你好世界，こんにちは、안녕하세요"
  "rtl|שלום עולם مرحبا"
  "combining|é ä ñ"
  "emoji|dictation 🎙️ ships 🚀 clean ✅"
  "pangram|The quick brown fox jumps over the lazy dog; PACK MY BOX WITH FIVE DOZEN LIQUOR JUGS! 0123456789 (ok?)"
  "multichunk|$LONG"
)

pass=0
for case in "${CASES[@]}"; do
  name="${case%%|*}"
  text="$(printf '%b' "${case#*|}")"
  out="$WORK/$name.txt"
  : > "$out"
  # Raw, no-echo tty so each typed byte reaches `cat` immediately.
  foot --app-id "kaydence-inject-proof-$name" --title "Kaydence injection proof" \
    sh -c "stty -icanon -echo; exec cat > '$out'" &
  FOOT_PID=$!
  addr=""
  for _ in $(seq 1 100); do
    addr="$(hyprctl clients -j | jq -r --arg c "kaydence-inject-proof-$name" \
      '.[] | select(.class == $c) | .address' | head -n1)"
    [ -n "$addr" ] && break
    sleep 0.05
  done
  [ -n "$addr" ] || fail "$name: proof window never mapped"

  "$BIN" --type-into "$addr" "$text"
  # Let foot drain the pty into the file, then compare byte-for-byte.
  for _ in $(seq 1 40); do
    [ "$(cat "$out")" = "$text" ] && break
    sleep 0.05
  done
  kill "$FOOT_PID" 2>/dev/null || true
  wait "$FOOT_PID" 2>/dev/null || true
  FOOT_PID=""
  got="$(cat "$out")"
  if [ "$got" = "$text" ]; then
    echo "[proof] PASS $name ($(printf '%s' "$text" | wc -m) chars, sha256 $(printf '%s' "$text" | sha256sum | cut -c1-12))"
    pass=$((pass + 1))
  else
    echo "[proof] expected: $text" >&2
    echo "[proof] received: $got" >&2
    fail "$name: readback mismatch"
  fi
done
# Focus stolen mid-delivery (Pitfall P9 during typing): the shipped injector
# must stop, window A keeps a clean prefix, and window B receives at most one
# guard batch (4 keys).
STEAL_TEXT="$(printf 'Kaydence stops typing the moment focus moves elsewhere. %.0s' $(seq 1 8))"
open_sink() { # app-id out-file → echoes the window address
  foot --app-id "$1" --title "Kaydence injection proof" \
    sh -c "stty -icanon -echo; exec cat > '$2'" >/dev/null 2>&1 &
  local a=""
  for _ in $(seq 1 100); do
    a="$(hyprctl clients -j | jq -r --arg c "$1" '.[] | select(.class == $c) | .address' | head -n1)"
    [ -n "$a" ] && break; sleep 0.05
  done
  echo "$a"
}
: > "$WORK/steal-a.txt"; : > "$WORK/steal-b.txt"
ADDR_A="$(open_sink kaydence-inject-proof-steal-a "$WORK/steal-a.txt")"
ADDR_B="$(open_sink kaydence-inject-proof-steal-b "$WORK/steal-b.txt")"
[ -n "$ADDR_A" ] && [ -n "$ADDR_B" ] || fail "focus-steal windows never mapped"
"$BIN" --focus-steal "$ADDR_A" "$ADDR_B" "$STEAL_TEXT" || fail "focus-steal: typing did not stop on a focus change"
sleep 0.3
for c in kaydence-inject-proof-steal-a kaydence-inject-proof-steal-b; do
  pid="$(hyprctl clients -j | jq -r --arg c "$c" '.[] | select(.class == $c) | .pid' | head -n1)"
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
done
GOT_A="$(cat "$WORK/steal-a.txt")"; GOT_B="$(cat "$WORK/steal-b.txt")"
[ "${#GOT_A}" -lt "${#STEAL_TEXT}" ] || fail "focus-steal: window A got the whole text — typing never stopped"
case "$STEAL_TEXT" in "$GOT_A"*) ;; *) fail "focus-steal: window A text is not a clean prefix" ;; esac
[ "${#GOT_B}" -le 4 ] || fail "focus-steal: window B received ${#GOT_B} chars (> one 4-key batch)"
echo "[proof] PASS focus-steal: stopped after ${#GOT_A}/${#STEAL_TEXT} chars; window B got ${#GOT_B} (≤ 4)"
pass=$((pass + 1))

echo "[proof] RESULT: PASS ($pass/$(( ${#CASES[@]} + 1 )) cases: byte-exact typing + mid-delivery focus guard)"
