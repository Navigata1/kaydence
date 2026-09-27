# engine/ — ASR

## Owns
The `AsrEngine` trait and its implementations: Parakeet V3 (ONNX Runtime,
independent CPU lane), whisper.cpp base.en Q5_1 (measured three-OS P1 default),
whisper.cpp large-v3-turbo (optional quality lane),
BYOK cloud (Groq/Deepgram).
Streaming partial emission, engine selection/fallback chain, model lifecycle
(load/warm/unload), language detection handoff.

## Does not own
Model downloading/verification (models/ registry + settings/), audio capture,
any text mutation beyond what the model outputs (cleanup/ owns that).

## Invariants
1. Emit `Partial` events ≤300 ms behind speech when the active engine supports
   streaming; emit `RawFinal` within budget (root AGENTS §5) after end-of-speech.
2. Fallback chain is mandatory and ordered: BYOK (if configured) → local GPU →
   local CPU. A cloud timeout (>2 s) silently falls back and logs locally;
   the user gets text, not an error.
3. Dictionary hints (biasing terms) are accepted as input where the engine
   supports them; ignore gracefully where not.
4. Engines are hot-swappable at runtime; switching never drops an in-flight session.
5. Raw model output is preserved verbatim into the session record — partial
   hallucination cleanup is cleanup/'s job, with the raw kept for diffing.
6. Model initialization never lands on first-dictation latency. Engines that
   load resident state implement `warm_up`; startup and idle-only adapter swaps
   warm the first viable lane, falling through only when initialization fails.
   Do not load the CPU fallback when the accelerated primary warmed successfully,
   and never retry a lane whose warmup failed on the transcription hot path.
7. Lane labels describe the backend that is compiled and selected, not the lane
   a caller requested. A build without an acceleration feature routes a requested
   GPU model to `LocalCpu` before warmup, transcription, candidate,
   required-model, preflight, runtime status, or evidence.
8. Shipped whisper.cpp binaries default `GGML_NATIVE=OFF`; build-host CPU
   detection is not a portable release target. Architecture-specific variants
   require an explicit target string plus runtime and latency evidence.
9. A platform first-run default must have direct evidence under the root
   `<=250 MB` ASR-resident budget on that platform. A larger quality model or
   another OS remains unpromoted until the same runner proves it; popularity or
   disk quantization alone is not footprint evidence.
10. One engine pass per dictation (ADR-0024). The pipeline joins the VAD's
    speech segments, silence trimmed to the gate's pre-roll, into passes of at
    most `pipeline::MAX_PASS_SECONDS`; a pause never costs a pass of its own.
    whisper.cpp fits its encoder window to that audio
    (`whisper::fitted_audio_ctx`) and never encodes less than the measured
    floor: narrower windows garbled short passes.

## Benchmarks
`scripts/bench.sh` drives this module with the golden audio corpus; WER and
latency per engine per platform land in `bench-results/` (gitignored, summary
in PR). A model upgrade PR must show the before/after table.

## Pitfalls
Whisper hallucinates on silence/noise — VAD gating upstream is the first
defense; reject finals whose no-speech probability exceeds threshold. ONNX
Runtime + Metal/DirectML feature flags differ per OS — keep provider selection
in one function with explicit logging of what was chosen. A generic "GPU"
request is not backend proof; capture the native runtime log naming Metal,
DirectML, CUDA, or the actual provider.
