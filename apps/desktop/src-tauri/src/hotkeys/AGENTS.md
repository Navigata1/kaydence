# hotkeys/ — Global Shortcuts

## Owns
Global hotkey registration (all three OSes: macOS/Windows/Linux X11+Wayland), push-to-talk vs toggle semantics,
debounce, the secondary per-invocation dial-override chord, conflict detection
with OS/app shortcuts, and emitting start/stop intents to the session manager.
On Wayland it also owns the compositor-bound hotkey (ADR-0023): the
`record start|stop|toggle|status` CLI (`kaydence-ctl`, `kaydence`) and the
private control socket (`control.rs`) that turns those verbs into the same
Press/Release edges a native grab produces.

## Invariants
1. **Timing discipline (pitfall P1):** debounce 30 ms; a press shorter than
   the 250 ms minimum capture window is treated as an accidental tap and
   discarded before downstream processing; release still carries a 300 ms tail
   buffer so real speech is not clipped.
2. Push-to-talk: key-down starts, key-up stops. Toggle: same key starts/stops
   with a hard 5-minute auto-stop safety. Both modes always available;
   default push-to-talk.
3. Registration failure (conflict) must surface in onboarding with a one-tap
   rebind — never silently dead.
4. macOS global monitoring requires Input Monitoring permission — degrade to
   an explanatory state, not a crash, when missing. Handy's macOS shortcut
   rewrite is the reference for the known OS quirks (ADR-0004).
5. Fn/Globe and media keys behave differently per OS/keyboard — keep a tested
   allowlist of recommended default chords (current default: hold Right-Alt /
   Right-Option; revisit in beta).
6. **Windows Right-Alt is served by Raw Input, not `global-hotkey` (ADR-0022).**
   `RegisterHotKey` cannot deliver a lone modifier, so `windows.rs` owns bare and
   Shift+Right-Alt; never "fix" it by mapping `AltRight` in the plugin (that
   registers but never fires). An owned press injects the `vkE8` menu mask so the
   target app keeps focus. A chord during the hold sends `Signal::Chord`: early →
   discarded like a tap, late → kept (never lose a word).
7. **The Raw Input listener sees every keystroke system-wide.** `raw_key.rs`
   reduces each record to a Right-Alt edge / Shift state / "other key" boolean
   and drops it. Key identities are never stored, logged, transmitted, or passed
   beyond `RightAltEdge`; changing that needs a new ADR.

8. **Wayland = compositor-bound, never a silent grab (ADR-0023).** An X11 grab
   inside a Wayland session only sees XWayland windows, so it never counts as
   "registered" there. Only a detected compositor binding or a received
   `record` command does. Control verbs map through the pure
   `control_signal`; they never bypass the coordinator. Socket-started
   captures carry the 5-minute safety stop. The socket stays same-user only,
   one verb in and one state word out, and never carries audio or text.
   Bindings call `kaydence-ctl` (no GUI libraries, ~2 ms exec). The full
   binary costs ~74 ms to exec, which exceeds the 50 ms hotkey budget.

## Tests
Rapid double-tap, hold-under-250ms, toggle auto-stop, rebind flow, conflict
detection fake. Control path: verb parsing, idempotent start/stop, short-tap
discard through the socket, and real-socket permission/peer/stale-socket tests.
Right-Alt reducer (auto-repeat, Shift role, AltGr/Alt+Tab chords, own mask
ignored) and early/late chord semantics run on every OS.
