#!/usr/bin/env bash
# check-adr-status.sh — P0-G7 gate.
#
# Every ADR must be Accepted before P0 closes, with declared by-design exceptions:
# ADR-0009 (Relay pairing/crypto) stays Proposed until the operator's crypto
# design review, which gates the P4-1 build — the one remaining exception.
# Former exceptions, removed on operator acceptance: ADR-0012 (visual identity,
# 2026-07-08), ADR-0013 (Linux injection strategy, its dependency-set spike),
# ADR-0015 (quantized P1 ASR default, 2026-07-13), ADR-0021 (race-free
# close-to-tray, 2026-09-24). This script passes when every
# ADR except the declared exceptions is Accepted.
#
# Portable to bash 3.2 (macOS default): no associative arrays, no mapfile.
#   usage: bash scripts/check-adr-status.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="${KAYDENCE_ADR_DIR:-$ROOT/docs/decisions}"

# ADR numbers allowed to be non-Accepted (space-separated), + reasons in one place.
PROPOSED_OK=" 0009 0016 0024 "
reason_for() { case "$1" in
  0009) echo "Relay crypto — critical decision path; operator review gates P4-1" ;;
  0016) echo "ONNX 2nd ASR lane + Silero VAD — operator go gates the vendored ORT binary + CTC model/CC-BY + source pins" ;;
  0024) echo "Fitted whisper encoder window — ASR behaviour change on every platform; operator go + cross-platform WER gate" ;;
  *)    echo "" ;;
esac; }

is_exception() { case "$PROPOSED_OK" in *" $1 "*) return 0;; *) return 1;; esac; }

fail=0
seen_numbers=" "
echo "== ADR status check =="
for f in "$DIR"/0[0-9][0-9][0-9]-*.md; do
  base="$(basename "$f")"
  num="${base%%-*}"
  [ "$num" = "0000" ] && continue   # template

  case "$seen_numbers" in
    *" $num "*)
      printf '  %-6s FAIL  (duplicate ADR number: %s)\n' "$num" "$base" >&2
      fail=1
      ;;
    *) seen_numbers="${seen_numbers}${num} " ;;
  esac

  # Status token = text after '**Status:**' and BEFORE any '<!--' comment.
  # (ADR-0009's comment literally contains the word "Accepted" — must not match it.)
  raw="$(grep -m1 -i 'Status:' "$f" || true)"
  token="${raw%%<!--*}"

  if printf '%s' "$token" | grep -qi 'Accepted'; then
    printf '  %-6s OK    (Accepted)\n' "$num"
  elif is_exception "$num"; then
    printf '  %-6s WAIVE (Proposed by design: %s)\n' "$num" "$(reason_for "$num")"
  else
    printf '  %-6s FAIL  (not Accepted: %s)\n' "$num" "$(printf '%s' "$token" | sed 's/^[[:space:]]*//')"
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "RESULT: FAIL — an ADR is not Accepted and not a declared exception." >&2
  exit 1
fi
echo "RESULT: PASS — all ADRs Accepted except declared by-design exceptions:$PROPOSED_OK"
exit 0
