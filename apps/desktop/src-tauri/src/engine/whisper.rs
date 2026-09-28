//! Real whisper.cpp ASR adapter (P1-P0-2, ADR-0014) — feature `asr-whisper`.
//!
//! Plugs into the existing `AsrEngine` trait and the `VerifiedArtifact` seam:
//! a checksum-verified LOCAL ggml model (path from `LocalAsrAdapterSpec`) is
//! loaded during runtime warmup (with lazy transcription fallback) and run over
//! the 16 kHz mono f32 the WAL already produces (`audio/wal.rs`). No network
//! surface — the model file is supplied locally; on-demand download stays a
//! separate gated decision (ADR-0014 §4).
//!
//! This whole module compiles only under `--features asr-whisper`, so the
//! default build stays free of the native whisper.cpp compile.

use super::{
    AsrEngine, AsrError, AsrRequest, AsrTranscript, AsrWarmup, EngineLane, LocalAsrAdapterSpec,
    PartialTranscript,
};
use crate::audio::wal::SAMPLE_RATE;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

/// whisper's encoder frames per second of audio (1500 frames = 30 s).
const ENCODER_FRAMES_PER_SECOND: f64 = 50.0;
/// Frames beyond the utterance itself. Measured on base.en q5_1 (ADR-0024):
/// +64 truncated or looped short clips ("S S S not."); +128 matched the full
/// window's words on every clip in the sweep.
const AUDIO_CTX_MARGIN: i32 = 128;
/// Never encode less than ~12.8 s. The pipeline's VAD hands whisper short
/// segments; a 1.47 s "ask not" segment read "S not." at 384 frames and
/// "as not." at 448–512, and was right from 576 up, with or without silence
/// padding. 640 keeps one step of headroom above that edge.
const AUDIO_CTX_FLOOR: i32 = 640;
/// The model's full window (30 s).
const AUDIO_CTX_MAX: i32 = 1500;
/// Fitted windows are rounded up to a multiple of 8, as voxtype does for its
/// GPU backends (Metal, Vulkan). The full window itself stays 1500, whisper's
/// unmodified default.
const AUDIO_CTX_ALIGN: i64 = 8;

/// Size whisper's encoder window to the utterance instead of always encoding a
/// fixed 30 s window (ADR-0024). A dictation is a few seconds long, and the
/// full window made one pass cost ~1.8 s on a laptop CPU; fitted, the same
/// pass costs ~0.6 s with the same words. Pure.
pub fn fitted_audio_ctx(samples: usize) -> i32 {
    let seconds = samples as f64 / f64::from(SAMPLE_RATE);
    let frames = (seconds * ENCODER_FRAMES_PER_SECOND).ceil() as i64;
    let wanted = frames.saturating_add(i64::from(AUDIO_CTX_MARGIN));
    let bounded = wanted.clamp(i64::from(AUDIO_CTX_FLOOR), i64::from(AUDIO_CTX_MAX));
    let aligned = (bounded + AUDIO_CTX_ALIGN - 1) / AUDIO_CTX_ALIGN * AUDIO_CTX_ALIGN;
    aligned.min(i64::from(AUDIO_CTX_MAX)) as i32
}

/// Why a fitted pass's output is not trusted. Any of these re-runs the pass
/// with the full window, which is exactly the unfitted behaviour, so a false
/// trigger costs time and never words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degenerate {
    /// No letter or digit from at least 1 s of audio: voxtype's
    /// `is_degenerate_transcript` rule (its #130 hardening).
    NoWords,
    /// The same phrase three or more times back to back, covering at least
    /// [`MIN_LOOP_WORDS`] words: the loop voxtype #124 reported for fitted
    /// windows ("Please increase that limit 5x." ×3).
    RepeatedPhrase,
    /// More words than anyone speaks in that much audio.
    TooManyWords,
}

impl std::fmt::Display for Degenerate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Degenerate::NoWords => "no words",
            Degenerate::RepeatedPhrase => "repeated phrase",
            Degenerate::TooManyWords => "too many words for the audio",
        })
    }
}

/// A loop must repeat at least this many words in total, so emphasis such as
/// "very very very" never counts as one.
const MIN_LOOP_WORDS: usize = 6;
/// Fast dictation runs 3–4.5 words per second; above 6 the text cannot all be speech.
const MAX_WORDS_PER_SECOND: f64 = 6.0;
/// Below this many words the rate is too noisy to judge.
const MIN_WORDS_FOR_RATE: usize = 12;

/// The measurable rule behind the full-window retry (ADR-0024). Pure.
pub fn degenerate_reason(text: &str, samples: usize) -> Option<Degenerate> {
    let seconds = samples as f64 / f64::from(SAMPLE_RATE);
    if seconds >= 1.0 && !text.chars().any(char::is_alphanumeric) {
        return Some(Degenerate::NoWords);
    }
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let n = words.len();
    for k in 1..=n / 3 {
        if 3 * k < MIN_LOOP_WORDS {
            continue;
        }
        for i in 0..=n - 3 * k {
            let unit = &words[i..i + k];
            if unit == &words[i + k..i + 2 * k] && unit == &words[i + 2 * k..i + 3 * k] {
                return Some(Degenerate::RepeatedPhrase);
            }
        }
    }
    if n >= MIN_WORDS_FOR_RATE && seconds > 0.0 && n as f64 / seconds > MAX_WORDS_PER_SECOND {
        return Some(Degenerate::TooManyWords);
    }
    None
}

/// A whisper.cpp-backed engine. Construction is infallible; explicit runtime
/// warmup loads the model before dictation, while `transcribe` retains a lazy
/// fallback for direct callers that do not run the lifecycle hook.
pub struct WhisperCppEngine {
    spec: LocalAsrAdapterSpec,
    ctx: Option<WhisperContext>,
    /// Reused across transcriptions: allocating whisper's decoding state cost
    /// 14–167 ms per call. Every pass runs with `no_context`, so nothing from
    /// one dictation can carry into the next.
    state: Option<WhisperState>,
}

impl WhisperCppEngine {
    pub fn boxed(spec: LocalAsrAdapterSpec) -> Box<dyn AsrEngine + Send> {
        Box::new(Self {
            spec,
            ctx: None,
            state: None,
        })
    }

    fn ensure_state(&mut self) -> Result<&mut WhisperState, AsrError> {
        if self.state.is_none() {
            let state = self
                .ensure_loaded()?
                .create_state()
                .map_err(|e| AsrError::Inference(format!("whisper create_state failed: {e}")))?;
            self.state = Some(state);
        }
        self.state.as_mut().ok_or_else(|| {
            AsrError::Unavailable("whisper state was not retained after creation".to_string())
        })
    }

    fn ensure_loaded(&mut self) -> Result<&WhisperContext, AsrError> {
        if self.ctx.is_none() {
            let path = self.spec.artifact_path.to_string_lossy().to_string();
            // Lane-aware acceleration: the LocalCpu lane forces GPU off so the
            // same model serves as the real CPU fallback when the GPU lane is
            // unavailable (engine/AGENTS.md fallback chain). LocalGpu/BYOK use
            // GPU where the build supports it.
            let mut params = WhisperContextParameters::default();
            params.use_gpu(!matches!(self.spec.lane, EngineLane::LocalCpu));
            let ctx = WhisperContext::new_with_params(&path, params).map_err(|e| {
                AsrError::Unavailable(format!(
                    "failed to load whisper model '{}' at {}: {e}",
                    self.spec.model_id, path
                ))
            })?;
            self.ctx = Some(ctx);
        }
        self.ctx.as_ref().ok_or_else(|| {
            AsrError::Unavailable("whisper context was not retained after loading".to_string())
        })
    }
}

impl WhisperCppEngine {
    /// One whisper pass with the given encoder window. Returns the coarse
    /// per-segment partials and the joined final text (possibly empty).
    fn run_pass(
        &mut self,
        samples: &[f32],
        audio_ctx: i32,
        prompt: &str,
    ) -> Result<(Vec<PartialTranscript>, String), AsrError> {
        let model_id = self.spec.model_id.clone();
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_audio_ctx(audio_ctx);
        // The state is reused across dictations: never feed one transcript to
        // the next as a prompt.
        params.set_no_context(true);
        // Keep whisper.cpp silent on stdout/stderr — this is a library path.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // Dictionary hints bias recognition via the initial prompt (engine/AGENTS.md
        // invariant 3: accept hints where the engine supports them).
        if !prompt.is_empty() {
            params.set_initial_prompt(prompt);
        }

        let state = self.ensure_state()?;
        if let Err(e) = state.full(params, samples) {
            // Never reuse a state that failed mid-pass.
            self.state = None;
            return Err(AsrError::Inference(format!(
                "whisper inference failed on {model_id}: {e}"
            )));
        }

        let num_segments = state.full_n_segments();
        let mut partials = Vec::new();
        let mut final_text = String::new();
        for i in 0..num_segments {
            let segment = state
                .get_segment(i)
                .ok_or_else(|| AsrError::Inference(format!("whisper segment {i} missing")))?;
            let seg = segment.to_str().map_err(|e| {
                AsrError::Inference(format!("whisper segment {i} text failed: {e}"))
            })?;
            let trimmed = seg.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !final_text.is_empty() {
                final_text.push(' ');
            }
            final_text.push_str(trimmed);
            // Emit the accumulated text as a coarse partial per segment; real
            // streaming partials land with the streaming path (deferred, ADR-0014).
            partials.push(PartialTranscript {
                text: final_text.clone(),
                t_lag_ms: 0,
            });
        }
        Ok((partials, final_text.trim().to_string()))
    }
}

impl AsrEngine for WhisperCppEngine {
    fn lane(&self) -> EngineLane {
        self.spec.lane
    }

    fn warm_up(&mut self) -> Result<AsrWarmup, AsrError> {
        // Load the model AND allocate the decoding state here, so neither
        // lands on the first dictation (engine invariant 6).
        self.ensure_state().map(|_| AsrWarmup::Ready)
    }

    fn transcribe(&mut self, request: &AsrRequest) -> Result<AsrTranscript, AsrError> {
        // whisper.cpp expects 16 kHz mono f32 — exactly the WAL/engine boundary
        // (audio/AGENTS.md). Reject anything else rather than silently mis-decode.
        if request.sample_rate != SAMPLE_RATE {
            return Err(AsrError::Inference(format!(
                "whisper adapter requires {SAMPLE_RATE} Hz mono f32, got {} Hz",
                request.sample_rate
            )));
        }

        let prompt = request.dictionary_hints.join(", ");
        let fitted = fitted_audio_ctx(request.samples.len());
        let (mut partials, mut final_text) = self.run_pass(&request.samples, fitted, &prompt)?;
        if fitted < AUDIO_CTX_MAX {
            if let Some(reason) = degenerate_reason(&final_text, request.samples.len()) {
                eprintln!(
                    "{} ASR {}: fitted pass (audio_ctx={fitted}) looked degenerate ({reason}); \
                     re-running with the full window",
                    crate::settings::APP_NAME,
                    request.id.0
                );
                (partials, final_text) = self.run_pass(&request.samples, AUDIO_CTX_MAX, &prompt)?;
            }
        }

        if final_text.is_empty() {
            return Err(AsrError::EmptyTranscript);
        }

        Ok(AsrTranscript {
            partials,
            final_text,
            no_speech_probability: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f64) -> usize {
        (s * f64::from(SAMPLE_RATE)) as usize
    }

    #[test]
    fn short_dictation_gets_the_floor_window() {
        // Up to ~10 s of speech: never below ~12.8 s of encoder context.
        assert_eq!(fitted_audio_ctx(0), AUDIO_CTX_FLOOR);
        assert_eq!(fitted_audio_ctx(secs(1.47)), AUDIO_CTX_FLOOR); // the "ask not" segment
        assert_eq!(fitted_audio_ctx(secs(4.9)), AUDIO_CTX_FLOOR); // 245 + 128 = 373
        assert_eq!(fitted_audio_ctx(secs(10.0)), AUDIO_CTX_FLOOR); // 500 + 128 = 628
    }

    #[test]
    fn longer_dictation_scales_with_its_length_plus_margin() {
        // 550 + 128 = 678, rounded up to a multiple of 8.
        assert_eq!(fitted_audio_ctx(secs(11.0)), 680);
        assert_eq!(fitted_audio_ctx(secs(20.0)), 1000 + AUDIO_CTX_MARGIN);
    }

    #[test]
    fn every_window_is_a_multiple_of_8_or_the_full_window() {
        for centis in 0..=3_000 {
            let ctx = fitted_audio_ctx(secs(f64::from(centis) / 100.0));
            assert!(ctx % 8 == 0 || ctx == AUDIO_CTX_MAX, "{centis} cs -> {ctx}");
            assert!((AUDIO_CTX_FLOOR..=AUDIO_CTX_MAX).contains(&ctx));
        }
    }

    const JFK: &str = "And so my fellow Americans asked not what your country can do \
                       for you, ask what you can do for your country.";

    #[test]
    fn a_looped_pass_is_degenerate() {
        // voxtype #124, verbatim: one phrase spoken once, transcribed three times.
        let looped =
            "Please increase that limit 5x. Please increase that limit 5x. Please increase that limit 5x.";
        assert_eq!(
            degenerate_reason(looped, secs(4.0)),
            Some(Degenerate::RepeatedPhrase)
        );
        // ADR-0024 sweep: the 6.23 s segment padded to 7.23 s at ctx 384.
        let sweep_loop =
            ["What your country can do for you, ask what you can do for your country."; 6]
                .join(" ");
        assert_eq!(
            degenerate_reason(&sweep_loop, secs(7.23)),
            Some(Degenerate::RepeatedPhrase)
        );
    }

    #[test]
    fn a_garbled_pass_is_degenerate() {
        for garbled in ["", "   ", "...", "♪♪ ♪", " - \n", "—"] {
            assert_eq!(
                degenerate_reason(garbled, secs(2.0)),
                Some(Degenerate::NoWords),
                "{garbled:?}"
            );
        }
        // Too many words for the audio: 40 distinct words in 2 s.
        let dense = (0..40)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            degenerate_reason(&dense, secs(2.0)),
            Some(Degenerate::TooManyWords)
        );
    }

    #[test]
    fn real_dictation_is_not_degenerate() {
        assert_eq!(degenerate_reason(JFK, secs(11.0)), None);
        assert_eq!(degenerate_reason("Ask not!", secs(1.48)), None);
        assert_eq!(
            degenerate_reason("And so my fellow Americans.", secs(2.88)),
            None
        );
        // Emphasis is not a loop.
        assert_eq!(
            degenerate_reason("That was very very very good.", secs(2.0)),
            None
        );
        assert_eq!(
            degenerate_reason("No, no, no. Not that one.", secs(2.0)),
            None
        );
        // Under 1 s, voxtype's rule does not apply: a blip may carry no words.
        assert_eq!(degenerate_reason("", secs(0.5)), None);
        // A fast but real 15 words in 3 s (5 per second) stays trusted.
        let fast = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen";
        assert_eq!(degenerate_reason(fast, secs(3.0)), None);
    }

    #[test]
    fn never_exceeds_the_models_window() {
        assert_eq!(fitted_audio_ctx(secs(27.5)), AUDIO_CTX_MAX);
        assert_eq!(fitted_audio_ctx(secs(120.0)), AUDIO_CTX_MAX);
        assert_eq!(fitted_audio_ctx(usize::MAX), AUDIO_CTX_MAX);
    }

    #[test]
    fn window_grows_monotonically_with_audio() {
        let mut last = 0;
        for tenth in 0..400 {
            let ctx = fitted_audio_ctx(secs(f64::from(tenth) / 10.0));
            assert!(ctx >= last, "window shrank at {tenth} tenths of a second");
            last = ctx;
        }
    }
}
