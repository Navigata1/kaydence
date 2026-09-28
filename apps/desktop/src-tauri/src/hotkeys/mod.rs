//! Global hotkey capture coordination (push-to-talk + toggle). Tauri setup owns
//! OS registration and feeds this platform-agnostic **capture state machine** —
//! the production answer to
//! Pitfall P1 (hotkey races truncating short utterances) adopted from Handy's
//! `TranscriptionCoordinator` (ADR-0004 study): a single serialized owner with a
//! debounce, min-capture floor, and tail buffer. Modelled as a pure state
//! machine (time injected as `u64` ms) so it is unit-tested without threads or a
//! real clock.
//!
//! Illegal transitions are unrepresentable ([`CaptureState`]). No stage imports
//! another's internals — communicate only via `SessionEvent`.
#![allow(dead_code)]

pub mod control;
pub mod raw_key;

#[cfg(target_os = "windows")]
pub mod windows;

/// Which activation gesture the hotkey uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyMode {
    /// Hold to capture; release (plus tail) finalizes.
    PushToTalk,
    /// Press to start; press again to finalize. Releases are ignored.
    Toggle,
}

/// Capture timing rules. Defaults from PRD P0-1 + Pitfall P1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureConfig {
    /// Captures shorter than this are discarded as accidental taps.
    pub min_capture_ms: u64,
    /// Keep capturing this long after the stop gesture so trailing audio is not
    /// clipped.
    pub tail_buffer_ms: u64,
    /// Presses within this window of the last press are treated as key-bounce /
    /// auto-repeat and ignored (the anti-double-start guard).
    pub debounce_ms: u64,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            min_capture_ms: 250,
            tail_buffer_ms: 300,
            debounce_ms: 30,
        }
    }
}

/// The capture lifecycle. Only these states exist, so a "double start" or a
/// "stop while idle" cannot be represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    /// Not capturing.
    Idle,
    /// Actively capturing since `started_ms`; `last_press_ms` powers debounce.
    Capturing { started_ms: u64, last_press_ms: u64 },
    /// Stop gesture seen; still capturing the tail until `ends_ms`.
    Finalizing { started_ms: u64, ends_ms: u64 },
}

/// An input edge fed to the coordinator (from the OS hotkey layer or a timer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Press {
        at_ms: u64,
    },
    Release {
        at_ms: u64,
    },
    /// A timer tick that drives the tail-buffer close.
    Tick {
        at_ms: u64,
    },
    /// The hold turned into a keyboard chord (e.g. AltGr text, Alt+Tab): stop
    /// now in either mode. Under the min-capture floor it is discarded like an
    /// accidental tap; past it the audio is kept (never lose a word). ADR-0022.
    Chord {
        at_ms: u64,
    },
}

/// The effect the runtime should perform in response to a signal. The runtime
/// owns audio/WAL; the coordinator only decides *when* to start/stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do (state may still have changed internally).
    None,
    /// Begin capture → audio thread on, WAL open (audio persists before ASR).
    StartCapture,
    /// Tail elapsed → stop capture and hand the session downstream.
    FinalizeCapture,
    /// Capture was too short (< `min_capture_ms`) → drop it as an accidental tap.
    DiscardCapture,
}

/// The single serialized owner of capture state. One instance processes every
/// hotkey edge in order — the Pitfall-P1 guarantee that rapid presses cannot
/// start overlapping captures.
pub struct CaptureCoordinator {
    mode: HotkeyMode,
    cfg: CaptureConfig,
    state: CaptureState,
}

impl CaptureCoordinator {
    pub fn new(mode: HotkeyMode, cfg: CaptureConfig) -> Self {
        Self {
            mode,
            cfg,
            state: CaptureState::Idle,
        }
    }

    pub fn state(&self) -> CaptureState {
        self.state
    }

    pub fn mode(&self) -> HotkeyMode {
        self.mode
    }

    /// Abort the current session after a runtime start failure. Normal stop
    /// paths should still flow through [`Signal`] so timing invariants apply.
    pub fn reset(&mut self) {
        self.state = CaptureState::Idle;
    }

    /// Process one signal; returns the runtime action and updates state.
    pub fn step(&mut self, sig: Signal) -> Action {
        use CaptureState::*;
        use HotkeyMode::*;
        use Signal::*;
        match (self.mode, self.state, sig) {
            // Start from idle.
            (_, Idle, Press { at_ms }) => {
                self.state = Capturing {
                    started_ms: at_ms,
                    last_press_ms: at_ms,
                };
                Action::StartCapture
            }
            // Key-bounce / auto-repeat while capturing: ignore, but coalesce the
            // press time so a genuine later press is measured from here.
            (
                _,
                Capturing {
                    started_ms,
                    last_press_ms,
                },
                Press { at_ms },
            ) if at_ms.saturating_sub(last_press_ms) < self.cfg.debounce_ms => {
                self.state = Capturing {
                    started_ms,
                    last_press_ms: at_ms,
                };
                Action::None
            }
            // Toggle: a real second press (past debounce) stops.
            (Toggle, Capturing { started_ms, .. }, Press { at_ms }) => {
                self.begin_finalize(started_ms, at_ms)
            }
            // Push-to-talk: extra presses while held are ignored.
            (PushToTalk, Capturing { .. }, Press { .. }) => Action::None,
            // Push-to-talk release: min-capture check, then finalize with tail.
            (PushToTalk, Capturing { started_ms, .. }, Release { at_ms }) => {
                self.begin_finalize(started_ms, at_ms)
            }
            // Toggle ignores releases entirely.
            (Toggle, _, Release { .. }) => Action::None,
            // A chorded hold stops now in either mode; the min-capture floor
            // decides discard vs keep, exactly as for a push-to-talk release.
            (_, Capturing { started_ms, .. }, Chord { at_ms }) => {
                self.begin_finalize(started_ms, at_ms)
            }
            // Tail window elapsed → finalize.
            (_, Finalizing { ends_ms, .. }, Tick { at_ms }) if at_ms >= ends_ms => {
                self.state = Idle;
                Action::FinalizeCapture
            }
            // Any other edge during finalizing (incl. an early re-press) is held
            // off until this session closes — keeps sessions from overlapping.
            (_, Finalizing { .. }, _) => Action::None,
            // Idle release / stray ticks.
            (_, _, _) => Action::None,
        }
    }

    /// Enter the tail-buffer window, or discard if the capture was too short.
    fn begin_finalize(&mut self, started_ms: u64, at_ms: u64) -> Action {
        if at_ms.saturating_sub(started_ms) < self.cfg.min_capture_ms {
            self.state = CaptureState::Idle;
            Action::DiscardCapture
        } else {
            self.state = CaptureState::Finalizing {
                started_ms,
                ends_ms: at_ms + self.cfg.tail_buffer_ms,
            };
            Action::None
        }
    }
}

// ───────────────────── compositor-bound control commands ────────────────────
//
// On Wayland an app cannot grab a global key; the compositor owns the keyboard
// (ADR-0023). Omarchy/Hyprland, Sway, and GNOME/KDE custom shortcuts instead
// *run a command* on press — and Hyprland/Sway also on release. So the Linux
// hotkey is `<app> record start|stop|toggle`, delivered over the local control
// socket (`control.rs`) and translated here into the SAME press/release edges the
// X11/macOS/Windows grab produces. Every timing invariant above (debounce,
// 250 ms floor, 300 ms tail) therefore applies unchanged.

/// A control command received from the compositor binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCommand {
    /// Key-down of a push-to-talk binding (or "start" of a toggle).
    Start,
    /// Key-up of a push-to-talk binding (or "stop" of a toggle).
    Stop,
    /// One-key toggle (GNOME/KDE custom shortcuts only fire on press).
    Toggle,
    /// Read-only state query (for status bars). Never changes capture state.
    Status,
}

impl ControlCommand {
    /// Parse the wire/CLI verb. Accepts voxtype-compatible verbs so an Omarchy
    /// user can swap `voxtype record start` for ours in the same binding.
    pub fn parse(verb: &str) -> Option<Self> {
        match verb.trim() {
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "toggle" => Some(Self::Toggle),
            "status" => Some(Self::Status),
            _ => None,
        }
    }

    pub fn verb(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Toggle => "toggle",
            Self::Status => "status",
        }
    }
}

/// Translate a control command into the hotkey edge it stands for, given the
/// current mode + state. `None` = nothing to do (already in the requested state,
/// or a status query). Idempotent by construction: a duplicated `start` (e.g. a
/// compositor auto-repeat) cannot restart or double-start a capture. Pure.
pub fn control_signal(
    mode: HotkeyMode,
    state: CaptureState,
    cmd: ControlCommand,
    at_ms: u64,
) -> Option<Signal> {
    use CaptureState::*;
    use ControlCommand::*;
    let press = Signal::Press { at_ms };
    // The edge that ends a capture differs by mode: push-to-talk ends on key-up,
    // toggle mode ends on a second press (releases are ignored there).
    let stop_edge = match mode {
        HotkeyMode::PushToTalk => Signal::Release { at_ms },
        HotkeyMode::Toggle => press,
    };
    match (cmd, state) {
        (Status, _) => None,
        (Start | Toggle, Idle) => Some(press),
        (Stop | Toggle, Capturing { .. }) => Some(stop_edge),
        // Start while capturing, stop while idle, anything while finalizing.
        _ => None,
    }
}

/// Stable lowercase label for the control socket's `status` reply (consumed by
/// status bars; never includes transcript content — privacy #1). Pure.
pub fn status_label(state: CaptureState) -> &'static str {
    match state {
        CaptureState::Idle => "idle",
        CaptureState::Capturing { .. } => "recording",
        CaptureState::Finalizing { .. } => "finalizing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptt() -> CaptureCoordinator {
        CaptureCoordinator::new(HotkeyMode::PushToTalk, CaptureConfig::default())
    }
    fn toggle() -> CaptureCoordinator {
        CaptureCoordinator::new(HotkeyMode::Toggle, CaptureConfig::default())
    }

    #[test]
    fn ptt_starts_on_press_and_finalizes_after_tail() {
        let mut c = ptt();
        assert_eq!(c.step(Signal::Press { at_ms: 0 }), Action::StartCapture);
        assert_eq!(c.step(Signal::Release { at_ms: 400 }), Action::None); // >= min → tail
        assert!(matches!(
            c.state(),
            CaptureState::Finalizing { ends_ms: 700, .. }
        ));
        assert_eq!(c.step(Signal::Tick { at_ms: 699 }), Action::None); // tail not up
        assert_eq!(c.step(Signal::Tick { at_ms: 700 }), Action::FinalizeCapture);
        assert_eq!(c.state(), CaptureState::Idle);
    }

    #[test]
    fn debounce_prevents_double_start() {
        // The Pitfall-P1 race: a bounced/repeated press must not start a 2nd capture.
        let mut c = ptt();
        assert_eq!(c.step(Signal::Press { at_ms: 0 }), Action::StartCapture);
        assert_eq!(c.step(Signal::Press { at_ms: 10 }), Action::None); // within 30ms → ignored
        assert!(matches!(
            c.state(),
            CaptureState::Capturing { started_ms: 0, .. }
        ));
    }

    #[test]
    fn short_tap_is_discarded() {
        let mut c = ptt();
        c.step(Signal::Press { at_ms: 0 });
        // Held only 100ms (< 250 floor) → accidental tap, dropped.
        assert_eq!(
            c.step(Signal::Release { at_ms: 100 }),
            Action::DiscardCapture
        );
        assert_eq!(c.state(), CaptureState::Idle);
    }

    #[test]
    fn tail_buffer_keeps_capturing_after_release() {
        let mut c = ptt();
        c.step(Signal::Press { at_ms: 0 });
        c.step(Signal::Release { at_ms: 300 }); // finalizing until 600
        assert!(matches!(
            c.state(),
            CaptureState::Finalizing { ends_ms: 600, .. }
        ));
        // still capturing through the tail — no finalize before the window closes
        assert_eq!(c.step(Signal::Tick { at_ms: 599 }), Action::None);
        assert_eq!(c.step(Signal::Tick { at_ms: 600 }), Action::FinalizeCapture);
    }

    #[test]
    fn toggle_starts_and_stops_on_presses_ignoring_release() {
        let mut c = toggle();
        assert_eq!(c.step(Signal::Press { at_ms: 0 }), Action::StartCapture);
        assert_eq!(c.step(Signal::Release { at_ms: 50 }), Action::None); // release ignored
        assert!(matches!(c.state(), CaptureState::Capturing { .. }));
        // second real press (past debounce, past min) → tail then finalize
        assert_eq!(c.step(Signal::Press { at_ms: 500 }), Action::None);
        assert_eq!(c.step(Signal::Tick { at_ms: 800 }), Action::FinalizeCapture);
    }

    #[test]
    fn early_repress_during_finalize_does_not_overlap() {
        let mut c = ptt();
        c.step(Signal::Press { at_ms: 0 });
        c.step(Signal::Release { at_ms: 300 }); // finalizing until 600
                                                // a fresh press at 350 must not start a second capture mid-finalize
        assert_eq!(c.step(Signal::Press { at_ms: 350 }), Action::None);
        assert!(matches!(c.state(), CaptureState::Finalizing { .. }));
    }

    #[test]
    fn runtime_can_reset_after_start_failure() {
        let mut c = ptt();
        assert_eq!(c.step(Signal::Press { at_ms: 0 }), Action::StartCapture);

        c.reset();

        assert_eq!(c.state(), CaptureState::Idle);
        assert_eq!(c.step(Signal::Press { at_ms: 500 }), Action::StartCapture);
    }

    // ── compositor-bound control commands (ADR-0023) ──

    /// Drive a coordinator purely through control commands, the way a
    /// compositor binding would.
    fn drive(c: &mut CaptureCoordinator, cmd: ControlCommand, at_ms: u64) -> Action {
        match control_signal(c.mode, c.state(), cmd, at_ms) {
            Some(sig) => c.step(sig),
            None => Action::None,
        }
    }

    #[test]
    fn control_verbs_parse_and_round_trip() {
        for cmd in [
            ControlCommand::Start,
            ControlCommand::Stop,
            ControlCommand::Toggle,
            ControlCommand::Status,
        ] {
            assert_eq!(ControlCommand::parse(cmd.verb()), Some(cmd));
        }
        assert_eq!(
            ControlCommand::parse(" start\n"),
            Some(ControlCommand::Start)
        );
        assert_eq!(ControlCommand::parse("START"), None);
        assert_eq!(ControlCommand::parse("rm -rf"), None);
        assert_eq!(ControlCommand::parse(""), None);
    }

    #[test]
    fn push_to_talk_bind_start_stop_behaves_like_a_held_key() {
        // Hyprland `bind` → start on key-down, `bindr` → stop on key-up.
        let mut c = ptt();
        assert_eq!(
            drive(&mut c, ControlCommand::Start, 0),
            Action::StartCapture
        );
        assert_eq!(drive(&mut c, ControlCommand::Stop, 400), Action::None);
        assert!(matches!(
            c.state(),
            CaptureState::Finalizing { ends_ms: 700, .. }
        ));
        assert_eq!(c.step(Signal::Tick { at_ms: 700 }), Action::FinalizeCapture);
    }

    #[test]
    fn control_path_keeps_the_short_tap_floor() {
        // Pitfall P1 still applies through the socket: a 100 ms tap is discarded.
        let mut c = ptt();
        drive(&mut c, ControlCommand::Start, 0);
        assert_eq!(
            drive(&mut c, ControlCommand::Stop, 100),
            Action::DiscardCapture
        );
        assert_eq!(c.state(), CaptureState::Idle);
    }

    #[test]
    fn duplicate_start_and_stray_stop_are_idempotent() {
        let mut c = ptt();
        assert_eq!(drive(&mut c, ControlCommand::Stop, 0), Action::None); // idle stop
        assert_eq!(
            drive(&mut c, ControlCommand::Start, 10),
            Action::StartCapture
        );
        // compositor key-repeat re-sends start: must not restart the capture
        assert_eq!(drive(&mut c, ControlCommand::Start, 500), Action::None);
        assert!(matches!(
            c.state(),
            CaptureState::Capturing { started_ms: 10, .. }
        ));
    }

    #[test]
    fn toggle_command_works_in_push_to_talk_mode() {
        // GNOME/KDE custom shortcuts fire on press only → one key toggles.
        let mut c = ptt();
        assert_eq!(
            drive(&mut c, ControlCommand::Toggle, 0),
            Action::StartCapture
        );
        assert_eq!(drive(&mut c, ControlCommand::Toggle, 600), Action::None);
        assert!(matches!(c.state(), CaptureState::Finalizing { .. }));
        // a toggle during the tail cannot overlap sessions
        assert_eq!(drive(&mut c, ControlCommand::Toggle, 650), Action::None);
        assert_eq!(c.step(Signal::Tick { at_ms: 900 }), Action::FinalizeCapture);
    }

    #[test]
    fn stop_command_ends_a_toggle_mode_capture() {
        // In toggle mode releases are ignored, so `stop` must map to a press.
        let mut c = toggle();
        assert_eq!(
            drive(&mut c, ControlCommand::Start, 0),
            Action::StartCapture
        );
        assert_eq!(drive(&mut c, ControlCommand::Stop, 500), Action::None);
        assert!(matches!(c.state(), CaptureState::Finalizing { .. }));
    }

    #[test]
    fn status_never_changes_state() {
        let mut c = ptt();
        assert_eq!(drive(&mut c, ControlCommand::Status, 0), Action::None);
        assert_eq!(status_label(c.state()), "idle");
        drive(&mut c, ControlCommand::Start, 0);
        assert_eq!(drive(&mut c, ControlCommand::Status, 100), Action::None);
        assert_eq!(status_label(c.state()), "recording");
        drive(&mut c, ControlCommand::Stop, 400);
        assert_eq!(status_label(c.state()), "finalizing");
    }

    #[test]
    fn early_chord_is_discarded_like_a_tap_in_both_modes() {
        for mode in [HotkeyMode::PushToTalk, HotkeyMode::Toggle] {
            let mut c = CaptureCoordinator::new(mode, CaptureConfig::default());
            c.step(Signal::Press { at_ms: 0 });
            assert_eq!(c.step(Signal::Chord { at_ms: 120 }), Action::DiscardCapture);
            assert_eq!(c.state(), CaptureState::Idle);
        }
    }

    #[test]
    fn late_chord_keeps_the_words_in_both_modes() {
        for mode in [HotkeyMode::PushToTalk, HotkeyMode::Toggle] {
            let mut c = CaptureCoordinator::new(mode, CaptureConfig::default());
            c.step(Signal::Press { at_ms: 0 });
            // 5 s into dictation, a bumped key must not discard the speech.
            assert_eq!(c.step(Signal::Chord { at_ms: 5_000 }), Action::None);
            assert!(matches!(c.state(), CaptureState::Finalizing { .. }));
            assert_eq!(
                c.step(Signal::Tick { at_ms: 5_300 }),
                Action::FinalizeCapture
            );
        }
    }

    #[test]
    fn chord_is_a_no_op_when_idle_or_finalizing() {
        let mut c = ptt();
        assert_eq!(c.step(Signal::Chord { at_ms: 0 }), Action::None);
        c.step(Signal::Press { at_ms: 0 });
        c.step(Signal::Release { at_ms: 400 }); // finalizing until 700
        assert_eq!(c.step(Signal::Chord { at_ms: 450 }), Action::None);
        assert_eq!(c.step(Signal::Tick { at_ms: 700 }), Action::FinalizeCapture);
    }
}
