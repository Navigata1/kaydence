//! Event-sequence suite (typed pipeline contract).
//!
//! Proves the cross-module order that users rely on: target bind, WAL first,
//! ASR output, cleanup, then exactly one delivery outcome. The real Parakeet /
//! Whisper adapters still own P1-G2; this suite locks the orchestration contract
//! they must plug into.

use kaydence_lib::{
    audio::{
        self,
        vad::{EnergyVad, SpeechGateConfig},
        wal,
    },
    cleanup,
    engine::{AsrEngine, AsrError, AsrRequest, AsrTranscript, EngineLane, EngineStack},
    events::{AppRef, CleanupDial, HoldReason, InjectMethod, SessionEvent, SessionId, Stage},
    inject::{
        self, FieldKind, InjectError, InjectorCaps, KeystrokeChannel, TextInjector,
        UnknownFieldPolicy,
    },
    pipeline::{self, TranscriptionPipeline},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use ulid::Ulid;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kaydence-event-sequences-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn app(id: &str, name: &str) -> AppRef {
    AppRef {
        id: id.to_string(),
        name: name.to_string(),
    }
}

fn test_vad_config() -> SpeechGateConfig {
    SpeechGateConfig {
        sample_rate: 1_000,
        frame_samples: 2,
        pre_roll_samples: 4,
        end_silence_samples: 4,
    }
}

fn summary_for_samples(label: &str, samples: &[f32]) -> (audio::CaptureSessionSummary, PathBuf) {
    let app_data = temp_dir(label);
    let sessions = app_data.join("sessions");
    let id = SessionId::new(Ulid::new());
    let mut writer = wal::WalWriter::create(&sessions, &id.0.to_string()).expect("create wal");
    writer.append(samples).expect("append samples");
    let wal_path = writer.finalize().expect("finalize wal");
    (
        audio::CaptureSessionSummary {
            id,
            wal_path,
            samples_written: samples.len() as u64,
            dropped_input_samples: 0,
            started_ms: 10,
            finalized_ms: 420,
        },
        app_data,
    )
}

#[derive(Clone)]
struct ScriptedEngine {
    lane: EngineLane,
    outputs: VecDeque<Result<AsrTranscript, AsrError>>,
    attempts: Arc<Mutex<Vec<EngineLane>>>,
}

impl ScriptedEngine {
    fn boxed(
        lane: EngineLane,
        outputs: Vec<Result<AsrTranscript, AsrError>>,
        attempts: Arc<Mutex<Vec<EngineLane>>>,
    ) -> Box<dyn AsrEngine + Send> {
        Box::new(Self {
            lane,
            outputs: outputs.into(),
            attempts,
        })
    }
}

impl AsrEngine for ScriptedEngine {
    fn lane(&self) -> EngineLane {
        self.lane
    }

    fn transcribe(&mut self, _request: &AsrRequest) -> Result<AsrTranscript, AsrError> {
        self.attempts.lock().expect("attempts lock").push(self.lane);
        self.outputs
            .pop_front()
            .unwrap_or_else(|| Err(AsrError::Inference("script exhausted".to_string())))
    }
}

#[derive(Debug)]
struct ScriptedInjector {
    field: FieldKind,
    method: InjectMethod,
    delivered: Vec<String>,
}

impl ScriptedInjector {
    fn editable_native() -> Self {
        Self {
            field: FieldKind::Editable,
            method: InjectMethod::Native,
            delivered: Vec::new(),
        }
    }

    fn secure_native() -> Self {
        Self {
            field: FieldKind::Secure,
            method: InjectMethod::Native,
            delivered: Vec::new(),
        }
    }
}

impl TextInjector for ScriptedInjector {
    fn caps(&self) -> InjectorCaps {
        InjectorCaps {
            native_text_insert: self.method == InjectMethod::Native,
            keystroke: match self.method {
                InjectMethod::Keystroke => KeystrokeChannel::MacOsEvent,
                InjectMethod::ClipboardRestore => KeystrokeChannel::MacOsEvent,
                InjectMethod::Native => KeystrokeChannel::None,
            },
            clipboard: self.method == InjectMethod::ClipboardRestore,
        }
    }

    fn focused_field(&self) -> FieldKind {
        self.field
    }

    fn insert_native(&mut self, text: &str) -> Result<(), InjectError> {
        self.delivered.push(text.to_string());
        Ok(())
    }

    fn synth_text(&mut self, text: &str) -> Result<(), InjectError> {
        self.delivered.push(text.to_string());
        Ok(())
    }

    fn paste_clipboard(&mut self, text: &str) -> Result<(), InjectError> {
        self.delivered.push(text.to_string());
        Ok(())
    }
}

fn process_with_stack(
    label: &str,
    stack: EngineStack,
    dial: CleanupDial,
) -> (SessionId, Vec<SessionEvent>, PathBuf) {
    let (summary, app_data) = summary_for_samples(label, &[0.0, 0.0, 0.6, 0.6]);
    let mut pipeline = TranscriptionPipeline::new(EnergyVad::new(0.2), test_vad_config(), stack)
        .with_cleanup_dial(dial);
    let events = pipeline.process_capture(&summary).expect("process capture");
    (summary.id, events, app_data)
}

fn assert_event_names(events: &[SessionEvent], names: &[&str]) {
    let actual = events
        .iter()
        .map(|event| match event {
            SessionEvent::Started { .. } => "started",
            SessionEvent::AudioPersisted { .. } => "audio_persisted",
            SessionEvent::Partial { .. } => "partial",
            SessionEvent::RawFinal { .. } => "raw_final",
            SessionEvent::CleanFinal { .. } => "clean_final",
            SessionEvent::Injected { .. } => "injected",
            SessionEvent::Held { .. } => "held",
            SessionEvent::Failed { .. } => "failed",
            SessionEvent::PredictionOffered { .. } => "prediction_offered",
            SessionEvent::PredictionMerged { .. } => "prediction_merged",
            SessionEvent::PredictionDismissed { .. } => "prediction_dismissed",
            SessionEvent::PredictionStale { .. } => "prediction_stale",
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, names);
}

#[test]
fn event_sequences_happy_path_wal_asr_cleanup_then_native_inject() {
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let transcript = AsrTranscript {
        partials: vec![kaydence_lib::engine::PartialTranscript {
            text: "um ship".to_string(),
            t_lag_ms: 120,
        }],
        final_text: "um ship ship it comma now".to_string(),
        no_speech_probability: Some(0.04),
    };
    let stack = EngineStack::new(vec![ScriptedEngine::boxed(
        EngineLane::LocalCpu,
        vec![Ok(transcript)],
        Arc::clone(&attempts),
    )]);
    let (id, pipeline_events, app_data) = process_with_stack("happy", stack, CleanupDial::Light);
    let mut events = vec![SessionEvent::Started {
        id,
        target_app: app("com.example.editor", "Editor"),
        at_ms: 10,
    }];
    events.extend(pipeline_events);

    let committed = pipeline::committed_text(&events).expect("committed text");
    let mut injector = ScriptedInjector::editable_native();
    events.push(inject::inject_committed_text(
        &mut injector,
        committed.id,
        &committed.text,
        UnknownFieldPolicy::Lenient,
        false,
    ));

    assert_event_names(
        &events,
        &[
            "started",
            "audio_persisted",
            "partial",
            "raw_final",
            "clean_final",
            "injected",
        ],
    );
    assert_eq!(
        attempts.lock().expect("attempts").as_slice(),
        &[EngineLane::LocalCpu]
    );
    assert_eq!(committed.text, "Ship it, now.");
    assert_eq!(injector.delivered, vec!["Ship it, now."]);
    assert!(matches!(
        events.last(),
        Some(SessionEvent::Injected {
            id: event_id,
            method: InjectMethod::Native
        }) if *event_id == id
    ));
    let _ = std::fs::remove_dir_all(app_data);
}

#[test]
fn event_sequences_focus_change_holds_after_clean_final_without_injecting() {
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let stack = EngineStack::new(vec![ScriptedEngine::boxed(
        EngineLane::LocalCpu,
        vec![Ok(AsrTranscript::raw("hello from the original app"))],
        attempts,
    )]);
    let (id, pipeline_events, app_data) =
        process_with_stack("focus-change", stack, CleanupDial::Light);
    let mut events = vec![SessionEvent::Started {
        id,
        target_app: app("com.example.editor", "Editor"),
        at_ms: 10,
    }];
    events.extend(pipeline_events);
    let committed = pipeline::committed_text(&events).expect("committed text");

    let bound = inject::FocusTarget::detected(app("com.example.editor", "Editor"));
    let current = inject::FocusTarget::detected(app("com.example.mail", "Mail"));
    let event = match inject::verify_focus_binding(&bound, &current) {
        Ok(()) => panic!("focus unexpectedly matched"),
        Err(reason) => SessionEvent::Held {
            id: committed.id,
            reason,
        },
    };
    events.push(event);

    assert_event_names(
        &events,
        &[
            "started",
            "audio_persisted",
            "raw_final",
            "clean_final",
            "held",
        ],
    );
    assert!(matches!(
        events.last(),
        Some(SessionEvent::Held {
            id: event_id,
            reason: HoldReason::FocusChanged
        }) if *event_id == id
    ));
    let _ = std::fs::remove_dir_all(app_data);
}

#[test]
fn event_sequences_secure_field_holds_after_clean_final_without_delivery() {
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let stack = EngineStack::new(vec![ScriptedEngine::boxed(
        EngineLane::LocalCpu,
        vec![Ok(AsrTranscript::raw("password manager note"))],
        attempts,
    )]);
    let (id, pipeline_events, app_data) =
        process_with_stack("secure-field", stack, CleanupDial::Light);
    let mut events = vec![SessionEvent::Started {
        id,
        target_app: app("com.example.vault", "Vault"),
        at_ms: 10,
    }];
    events.extend(pipeline_events);

    let committed = pipeline::committed_text(&events).expect("committed text");
    let mut injector = ScriptedInjector::secure_native();
    events.push(inject::inject_committed_text(
        &mut injector,
        committed.id,
        &committed.text,
        UnknownFieldPolicy::Lenient,
        false,
    ));

    assert_event_names(
        &events,
        &[
            "started",
            "audio_persisted",
            "raw_final",
            "clean_final",
            "held",
        ],
    );
    assert!(injector.delivered.is_empty());
    assert!(matches!(
        events.last(),
        Some(SessionEvent::Held {
            id: event_id,
            reason: HoldReason::SecureField
        }) if *event_id == id
    ));
    let _ = std::fs::remove_dir_all(app_data);
}

#[test]
fn event_sequences_engine_fallback_still_produces_one_injected_final() {
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let stack = EngineStack::new(vec![
        ScriptedEngine::boxed(
            EngineLane::ByokCloud,
            vec![Err(AsrError::Timeout { after_ms: 2_000 })],
            Arc::clone(&attempts),
        ),
        ScriptedEngine::boxed(
            EngineLane::LocalGpu,
            vec![Err(AsrError::Unavailable("provider missing".to_string()))],
            Arc::clone(&attempts),
        ),
        ScriptedEngine::boxed(
            EngineLane::LocalCpu,
            vec![Ok(AsrTranscript::raw("local fallback worked"))],
            Arc::clone(&attempts),
        ),
    ]);
    let (id, pipeline_events, app_data) =
        process_with_stack("engine-fallback", stack, CleanupDial::Light);
    let mut events = vec![SessionEvent::Started {
        id,
        target_app: app("com.example.editor", "Editor"),
        at_ms: 10,
    }];
    events.extend(pipeline_events);
    let committed = pipeline::committed_text(&events).expect("committed text");
    let mut injector = ScriptedInjector::editable_native();
    events.push(inject::inject_committed_text(
        &mut injector,
        committed.id,
        &committed.text,
        UnknownFieldPolicy::Lenient,
        false,
    ));

    assert_eq!(
        attempts.lock().expect("attempts").as_slice(),
        &[
            EngineLane::ByokCloud,
            EngineLane::LocalGpu,
            EngineLane::LocalCpu
        ]
    );
    assert_event_names(
        &events,
        &[
            "started",
            "audio_persisted",
            "raw_final",
            "clean_final",
            "injected",
        ],
    );
    assert_eq!(injector.delivered, vec!["Local fallback worked."]);
    let _ = std::fs::remove_dir_all(app_data);
}

#[test]
fn event_sequences_full_cleanup_timeout_floor_still_injects_text() {
    let id = SessionId::new(Ulid::new());
    let raw = "uh write write the note period";
    let mut events = vec![
        SessionEvent::Started {
            id,
            target_app: app("com.example.mail", "Mail"),
            at_ms: 10,
        },
        SessionEvent::AudioPersisted {
            id,
            wal_path: "/tmp/kaydence-full-timeout-floor.wav".to_string(),
        },
        SessionEvent::RawFinal {
            id,
            text: raw.to_string(),
        },
    ];

    // The model-backed Full pass is not loaded yet. The production cleanup
    // floor must still emit usable text and must not insert a Failed event
    // between RawFinal and injection.
    events
        .push(cleanup::clean_final_event(id, raw, CleanupDial::Full).expect("full cleanup floor"));
    let committed = pipeline::committed_text(&events).expect("committed text");
    let mut injector = ScriptedInjector::editable_native();
    events.push(inject::inject_committed_text(
        &mut injector,
        committed.id,
        &committed.text,
        UnknownFieldPolicy::Lenient,
        false,
    ));

    assert_event_names(
        &events,
        &[
            "started",
            "audio_persisted",
            "raw_final",
            "clean_final",
            "injected",
        ],
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        SessionEvent::Failed {
            stage: Stage::Clean,
            ..
        }
    )));
    assert_eq!(committed.text, "Write the note.");
    assert_eq!(injector.delivered, vec!["Write the note."]);
}

#[test]
fn event_sequences_pause_mid_dictation_is_one_engine_pass_and_one_delivery() {
    // Speech, a pause long enough to close a VAD segment, more speech.
    let samples = [0.6, 0.6, 0.0, 0.0, 0.0, 0.0, 0.6, 0.6];
    let (summary, app_data) = summary_for_samples("pause", &samples);
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let stack = EngineStack::new(vec![ScriptedEngine::boxed(
        EngineLane::LocalCpu,
        vec![Ok(AsrTranscript::raw("ask not what your country can do"))],
        Arc::clone(&attempts),
    )]);
    let mut pipeline = TranscriptionPipeline::new(EnergyVad::new(0.2), test_vad_config(), stack);
    let mut events = pipeline.process_capture(&summary).expect("process capture");

    let committed = pipeline::committed_text(&events).expect("committed text");
    let mut injector = ScriptedInjector::editable_native();
    events.push(inject::inject_committed_text(
        &mut injector,
        committed.id,
        &committed.text,
        UnknownFieldPolicy::Lenient,
        false,
    ));

    // ADR-0024: both segments reach the engine as one pass, so the model reads
    // the sentence whole and a pause costs no extra pass.
    assert_eq!(
        attempts.lock().expect("attempts").as_slice(),
        &[EngineLane::LocalCpu]
    );
    assert_event_names(
        &events,
        &["audio_persisted", "raw_final", "clean_final", "injected"],
    );
    assert_eq!(
        injector.delivered,
        vec!["Ask not what your country can do."]
    );
    let _ = std::fs::remove_dir_all(app_data);
}
