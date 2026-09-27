//! Pipeline coordination across the typed stages.
//!
//! This module does not implement capture, VAD, or ASR itself. It connects the
//! already-owned pieces in the only order allowed by the product contract:
//! WAL on disk first, VAD gate second, engine third.

use crate::audio::{
    self,
    vad::{SpeechGate, SpeechGateConfig, SpeechSegment, VadDetector},
};
use crate::cleanup;
use crate::dictionary;
use crate::engine::{self, AsrError, AsrRequest, EngineLane, EngineStack};
use crate::events::{CleanupDial, SessionEvent, SessionId, Stage};

/// Longest audio one engine pass receives (ADR-0024). The gate's speech
/// segments are joined into one pass per dictation, so a mid-sentence pause no
/// longer costs a pass of its own and the model reads the sentence whole. A
/// batch closes at a segment boundary before it would exceed this; whisper's
/// window is 30 s.
pub const MAX_PASS_SECONDS: u32 = 25;

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("wal: {0}")]
    Wal(#[from] audio::wal::WalError),
    #[error("asr: {0}")]
    Asr(#[from] AsrError),
}

pub trait CaptureProcessor {
    fn warm_up(&mut self) -> Result<Option<EngineLane>, PipelineError> {
        Ok(None)
    }

    fn set_cleanup_dial(&mut self, cleanup_dial: CleanupDial);

    fn process_capture(
        &mut self,
        summary: &audio::CaptureSessionSummary,
    ) -> Result<Vec<SessionEvent>, PipelineError>;
}

pub struct TranscriptionPipeline<D> {
    detector: D,
    vad_config: SpeechGateConfig,
    engines: EngineStack,
    dictionary_hints: Vec<String>,
    dictionary: dictionary::DictionaryPass,
    cleanup_dial: CleanupDial,
}

impl<D> TranscriptionPipeline<D>
where
    D: VadDetector,
{
    pub fn new(detector: D, vad_config: SpeechGateConfig, engines: EngineStack) -> Self {
        Self {
            detector,
            vad_config,
            engines,
            dictionary_hints: Vec::new(),
            dictionary: dictionary::DictionaryPass::default(),
            cleanup_dial: CleanupDial::Light,
        }
    }

    pub fn with_dictionary_hints(mut self, dictionary_hints: Vec<String>) -> Self {
        self.dictionary_hints = dictionary_hints;
        self
    }

    pub fn with_dictionary_pass(mut self, dictionary: dictionary::DictionaryPass) -> Self {
        self.dictionary = dictionary;
        self
    }

    pub fn with_cleanup_dial(mut self, cleanup_dial: CleanupDial) -> Self {
        self.cleanup_dial = cleanup_dial;
        self
    }

    pub fn process_capture(
        &mut self,
        summary: &audio::CaptureSessionSummary,
    ) -> Result<Vec<SessionEvent>, PipelineError> {
        let wal_samples = audio::wal::read_samples(&summary.wal_path)?;
        let mut gate = SpeechGate::new(&mut self.detector, self.vad_config);
        let mut segments = gate.push_samples(&wal_samples.samples);
        if let Some(segment) = gate.flush() {
            segments.push(segment);
        }
        let max_pass_samples = MAX_PASS_SECONDS as usize * wal_samples.sample_rate as usize;

        let mut events = vec![summary.audio_persisted_event()];
        for segment in join_segments(segments, max_pass_samples) {
            let request = AsrRequest::new(
                summary.id,
                wal_samples.sample_rate,
                segment.start_sample,
                segment.samples,
                self.dictionary_hints.clone(),
            );
            match self.engines.transcribe(&request) {
                Ok(run) => {
                    for event in run.events {
                        let clean_event = match &event {
                            SessionEvent::RawFinal { id, text } => {
                                cleanup::clean_final_event(*id, text, self.cleanup_dial)
                                    .map(|event| self.dictionary.apply_event(event))
                            }
                            _ => None,
                        };
                        events.push(event);
                        if let Some(clean_event) = clean_event {
                            events.push(clean_event);
                        }
                    }
                }
                Err(err) => events.push(SessionEvent::Failed {
                    id: summary.id,
                    stage: Stage::Recognize,
                    error: err.to_string(),
                }),
            }
        }
        Ok(events)
    }
}

/// Join consecutive speech segments into passes of at most `max_samples`.
/// Each segment already starts with the gate's pre-roll, so the join keeps a
/// short natural gap and drops the rest of the silence. A segment longer than
/// `max_samples` stays a pass of its own. Pure.
pub fn join_segments(segments: Vec<SpeechSegment>, max_samples: usize) -> Vec<SpeechSegment> {
    let mut passes: Vec<SpeechSegment> = Vec::new();
    for segment in segments {
        match passes.last_mut() {
            Some(pass) if pass.samples.len() + segment.samples.len() <= max_samples => {
                pass.samples.extend(segment.samples);
            }
            _ => passes.push(segment),
        }
    }
    passes
}

impl<D> CaptureProcessor for TranscriptionPipeline<D>
where
    D: VadDetector,
{
    fn warm_up(&mut self) -> Result<Option<EngineLane>, PipelineError> {
        Ok(self.engines.warm_up()?)
    }

    fn set_cleanup_dial(&mut self, cleanup_dial: CleanupDial) {
        self.cleanup_dial = cleanup_dial;
    }

    fn process_capture(
        &mut self,
        summary: &audio::CaptureSessionSummary,
    ) -> Result<Vec<SessionEvent>, PipelineError> {
        TranscriptionPipeline::process_capture(self, summary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedText {
    pub id: SessionId,
    pub text: String,
}

pub fn committed_text(events: &[SessionEvent]) -> Option<CommittedText> {
    let mut clean_segments = Vec::new();
    let mut raw_segments = Vec::new();
    let mut id = None;

    for event in events {
        match event {
            SessionEvent::CleanFinal {
                id: event_id, text, ..
            } => {
                id = Some(*event_id);
                clean_segments.push(text.as_str());
            }
            SessionEvent::RawFinal { id: event_id, text } => {
                id.get_or_insert(*event_id);
                raw_segments.push(text.as_str());
            }
            _ => {}
        }
    }

    let segments = if clean_segments.is_empty() {
        raw_segments
    } else {
        clean_segments
    };
    let text = segments.join(" ").trim().to_string();
    let id = id?;
    (!text.is_empty()).then_some(CommittedText { id, text })
}

pub fn default_runtime_pipeline(
    selected_model_id: Option<&str>,
    selected_lane: Option<&str>,
) -> TranscriptionPipeline<crate::audio::vad::EnergyVad> {
    let engine_lane = match selected_lane {
        Some("gpu") => EngineLane::LocalGpu,
        _ => EngineLane::LocalCpu,
    };

    default_runtime_pipeline_with_asr(engine::LocalAsrAdapterState::Pending {
        selected_model_id: selected_model_id.map(str::to_string),
        lane: engine_lane,
    })
}

pub fn default_runtime_pipeline_with_asr(
    asr_state: engine::LocalAsrAdapterState,
) -> TranscriptionPipeline<crate::audio::vad::EnergyVad> {
    TranscriptionPipeline::new(
        crate::audio::vad::EnergyVad::new(0.01),
        SpeechGateConfig::default(),
        engine::local_asr_stack(asr_state),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{vad::EnergyVad, wal};
    use crate::dictionary::{DictionaryPass, DictionaryTerm, Snippet};
    use crate::engine::{AsrEngine, AsrTranscript, EngineLane};
    use crate::events::{CleanupDial, SessionId, Stage};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use ulid::Ulid;

    fn tmp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kaydence-pipeline-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn session_id() -> SessionId {
        SessionId::new(Ulid::new())
    }

    fn test_vad_config() -> SpeechGateConfig {
        SpeechGateConfig {
            sample_rate: 1_000,
            frame_samples: 2,
            pre_roll_samples: 4,
            end_silence_samples: 4,
        }
    }

    fn summary_for_samples(samples: &[f32]) -> (audio::CaptureSessionSummary, std::path::PathBuf) {
        let app_data = tmp();
        let dir = app_data.join("sessions");
        let id = SessionId::new(Ulid::new());
        let mut writer = wal::WalWriter::create(&dir, &id.0.to_string()).unwrap();
        writer.append(samples).unwrap();
        let wal_path = writer.finalize().unwrap();
        (
            audio::CaptureSessionSummary {
                id,
                wal_path,
                samples_written: samples.len() as u64,
                dropped_input_samples: 0,
                started_ms: 10,
                finalized_ms: 400,
            },
            app_data,
        )
    }

    struct QueueEngine {
        outputs: VecDeque<Result<AsrTranscript, AsrError>>,
        requests: Arc<Mutex<Vec<AsrRequest>>>,
    }

    struct WarmupProbeEngine {
        calls: Arc<Mutex<u32>>,
    }

    impl QueueEngine {
        fn boxed(
            outputs: Vec<Result<AsrTranscript, AsrError>>,
            requests: Arc<Mutex<Vec<AsrRequest>>>,
        ) -> Box<dyn AsrEngine + Send> {
            Box::new(Self {
                outputs: outputs.into(),
                requests,
            })
        }
    }

    impl AsrEngine for QueueEngine {
        fn lane(&self) -> EngineLane {
            EngineLane::LocalCpu
        }

        fn transcribe(&mut self, request: &AsrRequest) -> Result<AsrTranscript, AsrError> {
            self.requests.lock().unwrap().push(request.clone());
            self.outputs
                .pop_front()
                .unwrap_or_else(|| Err(AsrError::Inference("no scripted output".to_string())))
        }
    }

    impl AsrEngine for WarmupProbeEngine {
        fn lane(&self) -> EngineLane {
            EngineLane::LocalCpu
        }

        fn warm_up(&mut self) -> Result<crate::engine::AsrWarmup, AsrError> {
            *self.calls.lock().unwrap() += 1;
            Ok(crate::engine::AsrWarmup::Ready)
        }

        fn transcribe(&mut self, _request: &AsrRequest) -> Result<AsrTranscript, AsrError> {
            Ok(AsrTranscript::raw("ready"))
        }
    }

    fn pipeline(
        outputs: Vec<Result<AsrTranscript, AsrError>>,
        requests: Arc<Mutex<Vec<AsrRequest>>>,
    ) -> TranscriptionPipeline<EnergyVad> {
        let engines = EngineStack::new(vec![QueueEngine::boxed(outputs, requests)]);
        TranscriptionPipeline::new(EnergyVad::new(0.2), test_vad_config(), engines)
            .with_dictionary_hints(vec!["Kaydence".to_string()])
    }

    #[test]
    fn capture_processor_warmup_delegates_to_the_engine_stack() {
        let calls = Arc::new(Mutex::new(0));
        let mut pipeline = TranscriptionPipeline::new(
            EnergyVad::new(0.2),
            test_vad_config(),
            EngineStack::new(vec![Box::new(WarmupProbeEngine {
                calls: Arc::clone(&calls),
            })]),
        );

        let lane = CaptureProcessor::warm_up(&mut pipeline).unwrap();

        assert_eq!(lane, Some(EngineLane::LocalCpu));
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn persisted_audio_event_precedes_engine_and_clean_events_for_speech() {
        let (summary, app_data) = summary_for_samples(&[0.0, 0.0, 0.5, 0.5]);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut pipeline = pipeline(
            vec![Ok(AsrTranscript::raw("um hello captain"))],
            Arc::clone(&requests),
        );

        let events = pipeline.process_capture(&summary).unwrap();

        assert_eq!(events[0], summary.audio_persisted_event());
        assert_eq!(
            events[1],
            SessionEvent::RawFinal {
                id: summary.id,
                text: "um hello captain".to_string()
            }
        );
        assert_eq!(
            events[2],
            SessionEvent::CleanFinal {
                id: summary.id,
                text: "Hello captain.".to_string(),
                dial: CleanupDial::Light
            }
        );
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].sample_rate, wal::SAMPLE_RATE);
        assert_eq!(seen[0].start_sample, 0);
        assert_eq!(seen[0].dictionary_hints, vec!["Kaydence"]);
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn silence_only_capture_does_not_call_engine() {
        let (summary, app_data) = summary_for_samples(&[0.0; 8]);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut pipeline = pipeline(
            vec![Ok(AsrTranscript::raw("should not happen"))],
            Arc::clone(&requests),
        );

        let events = pipeline.process_capture(&summary).unwrap();

        assert_eq!(events, vec![summary.audio_persisted_event()]);
        assert!(requests.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn separated_speech_segments_share_one_engine_request() {
        let samples = [0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.6, 0.6];
        let (summary, app_data) = summary_for_samples(&samples);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut pipeline = pipeline(
            vec![Ok(AsrTranscript::raw("first second"))],
            Arc::clone(&requests),
        );

        let events = pipeline.process_capture(&summary).unwrap();

        assert_eq!(
            events,
            vec![
                summary.audio_persisted_event(),
                SessionEvent::RawFinal {
                    id: summary.id,
                    text: "first second".to_string()
                },
                SessionEvent::CleanFinal {
                    id: summary.id,
                    text: "First second.".to_string(),
                    dial: CleanupDial::Light
                },
            ]
        );
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].start_sample, 0);
        // Segment one (2 samples) + segment two (4 pre-roll + 2 speech).
        assert_eq!(seen[0].samples.len(), 8);
        assert_eq!(seen[0].samples[2..6], [0.0; 4]);
        let _ = std::fs::remove_dir_all(app_data);
    }

    fn segment(start_sample: u64, len: usize) -> SpeechSegment {
        SpeechSegment {
            start_sample,
            samples: vec![start_sample as f32; len],
        }
    }

    #[test]
    fn joined_passes_close_at_a_segment_boundary_before_the_cap() {
        let passes = join_segments(vec![segment(0, 2), segment(10, 2), segment(20, 2)], 5);
        assert_eq!(passes.len(), 2);
        assert_eq!(passes[0].start_sample, 0);
        assert_eq!(passes[0].samples, vec![0.0, 0.0, 10.0, 10.0]);
        assert_eq!(passes[1], segment(20, 2));
    }

    #[test]
    fn a_segment_longer_than_the_cap_is_its_own_pass() {
        let passes = join_segments(vec![segment(0, 2), segment(10, 9), segment(30, 2)], 5);
        assert_eq!(passes, vec![segment(0, 2), segment(10, 9), segment(30, 2)]);
        assert!(join_segments(Vec::new(), 5).is_empty());
    }

    #[test]
    fn raw_cleanup_dial_bypasses_clean_final_events() {
        let (summary, app_data) = summary_for_samples(&[0.0, 0.0, 0.5, 0.5]);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut pipeline = pipeline(
            vec![Ok(AsrTranscript::raw("um untouched raw"))],
            Arc::clone(&requests),
        )
        .with_cleanup_dial(CleanupDial::Raw);

        let events = pipeline.process_capture(&summary).unwrap();

        assert_eq!(
            events,
            vec![
                summary.audio_persisted_event(),
                SessionEvent::RawFinal {
                    id: summary.id,
                    text: "um untouched raw".to_string(),
                }
            ]
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn recognize_failure_is_recorded_after_audio_persisted() {
        let (summary, app_data) = summary_for_samples(&[0.0, 0.0, 0.5, 0.5]);
        let mut pipeline = TranscriptionPipeline::new(
            EnergyVad::new(0.2),
            test_vad_config(),
            EngineStack::empty(),
        );

        let events = pipeline.process_capture(&summary).unwrap();

        assert_eq!(events[0], summary.audio_persisted_event());
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1],
            SessionEvent::Failed {
                id: summary.id,
                stage: Stage::Recognize,
                error: "all configured ASR engines failed".to_string()
            }
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn default_runtime_pipeline_reports_pending_selected_asr_adapter() {
        let (summary, app_data) = summary_for_samples(&[0.0, 0.0, 0.5, 0.5]);
        let mut pipeline = default_runtime_pipeline(Some("fixture-asr"), Some("cpu"));

        let events = pipeline.process_capture(&summary).unwrap();

        assert_eq!(events[0], summary.audio_persisted_event());
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1],
            SessionEvent::Failed {
                id: summary.id,
                stage: Stage::Recognize,
                error: "all configured ASR engines failed: local_cpu: engine unavailable: local ASR adapter is waiting for a verified artifact for selected model 'fixture-asr'".to_string()
            }
        );
        assert!(committed_text(&events).is_none());
        let _ = std::fs::remove_dir_all(app_data);
    }

    // Gated: with `asr-onnx` the onnxruntime artifact routes to the real adapter
    // (ADR-0016); this asserts the DEFAULT build's honest pending boundary.
    #[cfg(not(feature = "asr-onnx"))]
    #[test]
    fn default_runtime_pipeline_reports_verified_artifact_adapter_boundary() {
        let (summary, app_data) = summary_for_samples(&[0.0, 0.0, 0.5, 0.5]);
        let mut pipeline =
            default_runtime_pipeline_with_asr(engine::LocalAsrAdapterState::VerifiedArtifact {
                spec: engine::LocalAsrAdapterSpec {
                    model_id: "fixture-asr".to_string(),
                    lane: EngineLane::LocalCpu,
                    runtime: "onnxruntime".to_string(),
                    artifact_path: app_data.join("models/fixture-asr.onnx"),
                    artifact_size_bytes: 3,
                },
            });

        let events = pipeline.process_capture(&summary).unwrap();

        assert_eq!(events[0], summary.audio_persisted_event());
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1],
            SessionEvent::Failed {
                id: summary.id,
                stage: Stage::Recognize,
                error: format!(
                    "all configured ASR engines failed: local_cpu: engine unavailable: verified onnxruntime ASR artifact for selected model 'fixture-asr' is ready at {} (3 bytes), but the runtime adapter is not implemented yet",
                    app_data.join("models/fixture-asr.onnx").display()
                )
            }
        );
        assert!(committed_text(&events).is_none());
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn committed_text_prefers_clean_segments() {
        let id = session_id();
        let events = vec![
            SessionEvent::RawFinal {
                id,
                text: "um first".to_string(),
            },
            SessionEvent::CleanFinal {
                id,
                text: "First.".to_string(),
                dial: CleanupDial::Light,
            },
            SessionEvent::RawFinal {
                id,
                text: "um second".to_string(),
            },
            SessionEvent::CleanFinal {
                id,
                text: "Second.".to_string(),
                dial: CleanupDial::Light,
            },
        ];

        assert_eq!(
            committed_text(&events),
            Some(CommittedText {
                id,
                text: "First. Second.".to_string()
            })
        );
    }

    #[test]
    fn committed_text_uses_raw_when_cleanup_is_absent() {
        let id = session_id();
        let events = vec![
            SessionEvent::RawFinal {
                id,
                text: "first raw".to_string(),
            },
            SessionEvent::RawFinal {
                id,
                text: "second raw".to_string(),
            },
        ];

        assert_eq!(
            committed_text(&events),
            Some(CommittedText {
                id,
                text: "first raw second raw".to_string()
            })
        );
    }

    #[test]
    fn dictionary_personalizes_clean_final_before_commit_selection() {
        let (summary, app_data) = summary_for_samples(&[0.0, 0.0, 0.5, 0.5]);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let dictionary = DictionaryPass::new()
            .with_terms(vec![DictionaryTerm::user("kaydence", "Kaydence")])
            .with_snippets(vec![Snippet::exact("sig", "Regards, Kaydence")]);
        let mut pipeline = pipeline(
            vec![Ok(AsrTranscript::raw("hello kaydence sig"))],
            Arc::clone(&requests),
        )
        .with_dictionary_pass(dictionary);

        let events = pipeline.process_capture(&summary).unwrap();

        assert_eq!(
            events[2],
            SessionEvent::CleanFinal {
                id: summary.id,
                text: "Hello Kaydence Regards, Kaydence.".to_string(),
                dial: CleanupDial::Light
            }
        );
        assert_eq!(
            committed_text(&events),
            Some(CommittedText {
                id: summary.id,
                text: "Hello Kaydence Regards, Kaydence.".to_string(),
            })
        );
        let _ = std::fs::remove_dir_all(app_data);
    }
}
