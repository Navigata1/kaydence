# ADR-0024: One whisper pass per dictation, with the encoder window fitted to it

- **Status:** Proposed <!-- Human gate: changes ASR behaviour on every platform (engine/ is frontier work). Listed in scripts/check-adr-status.sh PROPOSED_OK until the operator decides. -->
- **Date:** 2026-09-26
- **PRD items affected:** P0-2 (local ASR), P1-G3 (latency/footprint gate), root §5
  budget "key release → Raw final injected (local CPU) ≤ 1,200 ms"

## Context

The shipped whisper.cpp lane (base.en q5_1, ADR-0015) had two costs that scaled
badly on a laptop CPU. Measurements are from the Omarchy x86_64 reference laptop
(Intel i7-8650U, 4 threads, `GGML_NATIVE=OFF`).

1. **Every pass encoded a fixed 30 s window.** One pass cost ~1.8 s whether the
   speech was 1.4 s or 4.9 s long.
2. **Every pause cost another pass.** The pipeline ran the VAD over the whole
   recording after key release and sent each speech segment to the engine
   separately. The gate closes a segment after 300 ms of silence, so a normal
   sentence with one breath in it became 2–3 serial passes.

Together:
- the reference bench (4.9 s clip, 2 segments) measured release → delivery-policy
  **p50 3,606 ms / p95 4,062 ms** against the 1,200 ms budget;
- a live end-to-end dictation of an 11 s sentence (3 segments) took **~5,950 ms**
  from key release to text.

Allocating a fresh `WhisperState` per pass added another 14–167 ms.

## Decision

**Transcribe each dictation in one pass, size whisper's encoder window to that
pass, reuse one decoding state, and never carry context between dictations.**

1. **One pass per dictation (`pipeline::join_segments`).** The VAD still trims
   silence and skips silence-only captures. Its speech segments are then joined
   into one engine request. Each segment already starts with the gate's 250 ms
   pre-roll, so the join keeps a short natural gap. A pass closes at a segment
   boundary before it would exceed `MAX_PASS_SECONDS = 25` (whisper's window is
   30 s). A single segment longer than that stays a pass of its own, as before.
2. **`audio_ctx = clamp(⌈seconds × 50⌉ + 128, 640, 1500)`.** That is whisper's 50
   encoder frames per second of audio, plus a 128-frame (~2.6 s) margin, never
   below 640 frames (~12.8 s) and never above the model's 1500 (30 s). It is
   implemented as the pure `engine::whisper::fitted_audio_ctx`, with unit tests.
3. **One `WhisperState` per engine.** It is allocated during `warm_up` (engine
   invariant 6: no initialization on the first dictation) and reused. It is
   dropped and rebuilt after any failed pass.
4. **`no_context = true` on every pass.** One dictation's text can never become
   the next one's prompt. Dictionary hints still arrive through `initial_prompt`.

## Evidence (Omarchy x86_64, base.en q5_1)

**The floor is set by short passes.** The pipeline's own VAD (EnergyVad 0.01,
default gate) split the 11 s JFK clip into segments of 2.88 s, 1.47 s and 6.23 s.
Each was sent to whisper alone, with 0, 0.5 and 1 s of added silence:

| 1.47 s "ask not" segment | ctx 384 | 448 | 512 | 576 | 640 |
|---|---|---|---|---|---|
| no padding | "S not." | "as not." | "as not." | ✓ | ✓ |
| +0.5 s silence each side | ✓ | ✓ | ✓ | ✓ | ✓ |
| +1 s silence each side | "S not" | ✓ | ✓ | ✓ | ✓ |

The 6.23 s segment padded to 8.23 s fell into a repetition loop at ctx 384 and
448 (2.6–2.8 s). It was correct from 512 up. **640 is the lowest window correct
in every case, with one 64-frame step of headroom.** The first version of this
ADR used a 384 floor, measured on whole clips. That was wrong for the short
segments the real pipeline produces. The live end-to-end run caught it ("As not").

**One pass beats per-segment passes on both speed and words.** Same model,
reused state, fitted window with the 640 floor, 3 rounds:

| Clip | Per-segment passes | One joined pass |
|---|---|---|
| jfk.wav 11.0 s (3 segments) | 1,845–1,968 ms; "Ask not!" fragments; round 2 read "And so **am I** fellow Americans" | **700–761 ms**; one sentence, correct words, 3/3 |
| jfk-ask-not.wav 4.9 s (2 segments) | 1,192–1,261 ms; "Ask not! What your…" | **607–612 ms**; "Ask not what your country can do for you." 3/3 |
| clip16k.wav 4.9 s (2 segments) | 1,174–1,544 ms | **596–636 ms**; same words 3/3 |

**Project gates, before → after:**

| Gate | Unchanged adapter | This ADR |
|---|---|---|
| reference-bench p95 (budget 1,200 ms) | 4,062 ms FAIL | **742 / 636 ms PASS** (2 runs) |
| reference-bench p50 | 3,606 ms | 676 / 607 ms |
| idle RAM (budget 250 MB) | 107.6 MB | 146–147 MB (decoding state stays resident) |
| asr_golden, 11 s clip | 2,095 ms FAIL | **721 ms PASS**, same text |
| live E2E 4.9 s: key release → first character | ~4,055 ms | **971 ms**, "Ask not what your country can do for you." |
| live E2E 11 s: key release → first character | ~5,950 ms | **1,069 / 1,108 ms** |

The live figures include the app's 300 ms capture tail.

**One known word difference.** In the live 11 s run the whole sentence reads
"Americans **asked** not". That is this model's own reading of the full
sentence: the unchanged adapter produces the same text from this clip with the
full 30 s window (asr_golden; PR #57 evidence §10.4). The old per-segment path
only said "Ask not!" because it heard those two words alone. A file-fed joined
pass read "ask not" 3/3, so this word sits on the model's edge. A larger model
is the fix for it, not a wider window.

Raw logs: `ops/mission/evidence/2026-09-26-p1-g3-whisper-audio-ctx.txt`.

## Alternatives considered

- **Keep the full window and per-segment passes.** Blows the CPU budget 3–5×.
- **Fit the window but keep per-segment passes.** At the safe 640 floor every
  pause costs ~0.6 s. The bench measured p95 1,395 ms (FAIL). Isolated fragments
  also read worse ("Ask not! What your…", "And so am I").
- **A lower floor (384) with per-segment passes.** Passes the bench (p95 736 ms)
  but garbled the 1.47 s segment live ("As not"). Rejected: speed that types the
  wrong word is not a win.
- **Stream segments to the engine during capture.** This would hide ASR behind
  speech, but it changes the capture/WAL architecture. It stays a follow-up that
  composes with this decision.
- **A smaller model (tiny.en) or a GPU lane.** Tiny trades away quality, and many
  Linux laptops have no usable GPU lane. Both remain possible later.

## Consequences

- **Easier:** a normal dictation is one ~0.6–0.8 s pass on a laptop CPU, however
  many breaths it contains. The model sees the whole sentence, so punctuation and
  capitalization follow the sentence rather than the pauses. The macOS Metal and
  Windows ARM lanes also skip ~25 s of padded encoding per pass. Parakeet and
  BYOK engines receive the same joined pass.
- **Changed contract:** the pipeline now emits one `RawFinal` per pass, not per
  VAD segment. `committed_text` already joined segments with a space, so what is
  injected is unchanged in shape. The raw text in history is the model's verbatim
  output for the pass (engine invariant 5).
- **Harder / must verify before Accepted:**
  - the sweep covers one speaker and one model;
  - the same golden-corpus/WER check must pass on macOS (Metal + CPU) and
    Windows (ARM64 + x64), plus noisy, accented and long (> 25 s) dictation;
  - the margin, floor and pass cap are constants in `whisper.rs` and
    `pipeline/mod.rs` if a platform needs tuning.
- **Unchanged:** no new dependency, no network surface, no change to model files
  or the registry.
