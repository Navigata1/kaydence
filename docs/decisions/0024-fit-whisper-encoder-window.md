# ADR-0024: Fit whisper's encoder window to the utterance

- **Status:** Proposed <!-- Human gate: changes ASR behaviour on every platform (engine/ is frontier work). Listed in scripts/check-adr-status.sh PROPOSED_OK until the operator decides. -->
- **Date:** 2026-09-26
- **PRD items affected:** P0-2 (local ASR), P1-G3 (latency/footprint gate), root §5
  budget "key release → Raw final injected (local CPU) ≤ 1,200 ms"

## Context

The shipped whisper.cpp lane (base.en q5_1, ADR-0015) encodes a **fixed 30 s mel
window on every pass**, however short the dictation is. On the Omarchy x86_64 reference
laptop (Intel i7-8650U, 4 threads, `GGML_NATIVE=OFF`):
- one pass costs ~1.8 s whether the speech is 1.4 s or 4.9 s long;
- the reference bench (4.9 s clip, 10 samples) measured release → delivery-policy
  **p50 3,606 ms / p95 4,062 ms** against a 1,200 ms budget;
- a live end-to-end dictation took **4,055 ms** from key release to first typed
  character (PR #57 evidence §10.5, §10.11).

Allocating a fresh `WhisperState` per pass added another 14–167 ms.

## Decision

**Size the encoder window to the utterance, reuse one decoding state, and never carry
context between dictations.**

1. `audio_ctx = clamp(⌈seconds × 50⌉ + 128, 384, 1500)`: whisper's 50 frames per
   second of audio, plus a 128-frame (~2.6 s) margin, never below 384 frames (~7.7 s),
   never above the model's 1500 (30 s). Implemented as the pure
   `engine::whisper::fitted_audio_ctx`, with unit tests.
2. One `WhisperState` per engine, allocated during `warm_up` (engine invariant 6: no
   initialization on the first dictation) and reused. It is dropped and rebuilt after
   any failed pass.
3. `no_context = true` on every pass, so one dictation's text can never become the
   next one's prompt. Dictionary hints still arrive through `initial_prompt`.

## Evidence (Omarchy x86_64, base.en q5_1, fresh state per run, median of 3)

| Clip (JFK, human speech) | Full 30 s window | Fitted (this ADR) | Words |
|---|---|---|---|
| "and so my fellow Americans" 2.1 s | 1,732 ms¹ | **340 ms** | identical |
| "ask not" 1.4 s | 1,787 ms | **353 ms** | identical |
| "ask not" + 0.8 s silence each side, 3.0 s | 1,792 ms | **349 ms** | identical |
| "what your country can do for you" 2.5 s | 1,798 ms | **353 ms** | identical |
| "ask what you can do for your country" 3.0 s | 1,846 ms | **363 ms** | identical |
| "ask not what your country can do for you" 4.9 s | 1,869 ms | **447 ms** | identical |
| full quote, 11.0 s | 2,137 ms | **798 ms** | "asked not" vs "ask not"² |

Differences in punctuation and capitals only ("!" vs "."), except as noted.
¹ Median from the first sweep; this cell's fresh-state rerun was a 7.8 s outlier from
host contention.
² The adapter's own full-window golden run produced "asked not" on this same clip
(PR #57 evidence §10.4), so this is existing model variance at this word, not a new
error.

Before/after numbers from the project's own `reference-bench` and the live end-to-end
harness are recorded in `ops/mission/evidence/2026-09-26-p1-g3-whisper-audio-ctx.txt`.

## Alternatives considered

- **Keep the full window.** Blows the CPU budget by 3× on this class of laptop.
- **Tighter fits (+64 margin, or a 256 floor).** The same sweep produced "S S S not.",
  "americ and", "S to arms N", and repetition loops that took *longer* than the full
  window (2.5–3.8 s). Rejected.
- **Looser fit (+192, floor 512).** Also word-correct, but ~30 % slower than the chosen
  setting.
- **A smaller model (tiny.en) or a GPU lane.** Tiny trades away quality; many Linux
  laptops have no usable GPU lane. Both stay possible later and compose with this change.

## Consequences

- **Easier:** short dictations run ~5× faster in the ASR stage on laptop CPUs; the
  macOS Metal and Windows ARM lanes also skip ~25 s of padded encoding per pass.
- **Harder / must verify before Accepted:** the sweep is one speaker and one model. The
  same golden-corpus/WER check must pass on macOS (Metal + CPU) and Windows (ARM64 +
  x64), plus noisy and accented audio, before this is accepted. The margin and floor are
  constants in one place (`whisper.rs`) if a platform needs tuning.
- **Unchanged:** no new dependency, no network surface, no change to model files or the
  registry.
