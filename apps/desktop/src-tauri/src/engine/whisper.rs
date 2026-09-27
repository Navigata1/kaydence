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

/// Size whisper's encoder window to the utterance instead of always encoding a
/// fixed 30 s window (ADR-0024). A dictation is a few seconds long, and the
/// full window made one pass cost ~1.8 s on a laptop CPU; fitted, the same
/// pass costs ~0.6 s with the same words. Pure.
pub fn fitted_audio_ctx(samples: usize) -> i32 {
    let seconds = samples as f64 / f64::from(SAMPLE_RATE);
    let frames = (seconds * ENCODER_FRAMES_PER_SECOND).ceil() as i64;
    let wanted = frames.saturating_add(i64::from(AUDIO_CTX_MARGIN));
    wanted.clamp(i64::from(AUDIO_CTX_FLOOR), i64::from(AUDIO_CTX_MAX)) as i32
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

        let model_id = self.spec.model_id.clone();
        let dictionary_hints = request.dictionary_hints.clone();

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_audio_ctx(fitted_audio_ctx(request.samples.len()));
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
        let prompt;
        if !dictionary_hints.is_empty() {
            prompt = dictionary_hints.join(", ");
            params.set_initial_prompt(&prompt);
        }

        let state = self.ensure_state()?;
        if let Err(e) = state.full(params, &request.samples) {
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

        let final_text = final_text.trim().to_string();
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
        assert_eq!(fitted_audio_ctx(secs(11.0)), 550 + AUDIO_CTX_MARGIN);
        assert_eq!(fitted_audio_ctx(secs(20.0)), 1000 + AUDIO_CTX_MARGIN);
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
