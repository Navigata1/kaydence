# ADR-0022: Windows Right-Alt push-to-talk via Raw Input with menu masking

- **Status:** Proposed — the operator chose this approach ("Option A") on 2026-09-24
  after the live probes below; the full design (keystroke-visibility limits, chord
  stop, a new coordinator signal) awaits operator go on its PR. New platform input
  surface in `hotkeys/` = root AGENTS §7.5 critical decision path.
- **Date:** 2026-09-24
- **PRD items affected:** P1-P0-1 (global hotkey, push-to-talk + toggle); P1-P0-8
  (first run must not start with a broken default hotkey); hotkeys/AGENTS.md
  invariants 1–3 and 5.

## Context

The default binding is "hold Right-Alt" on every OS (hotkeys/AGENTS.md invariant 5).
On Windows it fails on every launch with "Unknown VKCode for AltRight":
`global-hotkey` 0.8's Windows `key_to_vk` has no `AltRight` arm, so a Windows first
run always starts with a registration failure and a rebind prompt.

Probes on a Windows 11 x64 NUC (en-US), one synthetic Right-Alt tap each
(`ops/mission/evidence/2026-09-24-p1-p0-1-windows-right-alt.txt`):

| Mechanism | Result |
|---|---|
| `RegisterHotKey(0, VK_RMENU)` (what mapping `AltRight` in the crate would do) | registers, **never fires** |
| `RegisterHotKey(MOD_ALT, VK_MENU)` | fires, but either Alt, and a registered hotkey swallows the key system-wide (Alt+Tab, menus) |
| Raw Input, `RIDEV_INPUTSINK` | clean right-side DOWN and UP in the background, no elevation |

So "just add the mapping" would turn a visible error into a silently dead hotkey,
which violates invariant 3. Raw Input works, and ARCHITECTURE §5 already names it
for Windows hotkeys, but it does not swallow the key. A lone Alt tap then activates
the focused app's menu: in Notepad, focus moved from the text document to the
`File` menu item, so Kaydence's own secure-field gate would Hold every dictation
(`NoTarget`). Injecting an unassigned key (`vkE8`) while Alt is held kept focus in
the document (same probe file).

## Decision

On Windows, serve the Right-Alt bindings from a **Raw Input listener** with
**menu-activation masking** and **chord stop**:

1. **Listener.** `hotkeys/windows.rs` runs one thread with a hidden, never-shown
   window registered for keyboard Raw Input with `RIDEV_INPUTSINK`. No hook, no
   elevation. The global-shortcut plugin never registers an `AltRight` shortcut on
   Windows; every other binding (F13, Ctrl+Space, …) stays on the plugin.
2. **Routing.** Right-Alt maps to a role through the same bound shortcuts
   `HotkeyRuntimeHandle` already holds (`AltRight` → Primary,
   `Shift+AltRight` → CleanupOverride). Rebinding away from Right-Alt makes the
   listener ignore it, and rebinding back needs no plugin registration.
3. **Masking.** When Kaydence owns a Right-Alt press, the listener immediately
   injects `vkE8` down/up (tagged via `dwExtraInfo` and ignored by the listener),
   so releasing Right-Alt does not activate the target app's menu. Presses
   Kaydence does not own are never masked.
4. **Chord stop.** If a non-modifier key goes down while Right-Alt is held (AltGr
   characters on non-US layouts, Alt+Tab), the hold is a chord, not push-to-talk.
   A new coordinator `Signal::Chord` stops the capture immediately in either mode
   (toggle included), and the existing 250 ms min-capture floor decides the
   outcome: an early chord is discarded like an accidental tap, so AltGr typing
   is never recorded as dictation; a late chord keeps and finalizes the audio, so
   a key bumped mid-dictation never destroys speech (non-negotiable #2).
5. **Keystroke visibility (non-negotiable #1).** Raw Input delivers every keyboard
   event system-wide to the listener, as any working option here would. Each
   record is reduced in memory to one of: Right-Alt edge, Shift state, or "another
   key went down", and then dropped. Key identities are never stored, logged,
   transmitted, or passed beyond that edge enum.
6. **Timing.** Edges are timestamped at receipt, so hold duration (250 ms minimum
   capture, invariant 1) is not skewed by runtime work.
7. **Failure is visible.** If Raw Input registration fails, a Right-Alt binding
   fails registration the same way any shortcut does (one-tap rebind, invariant 3).

Dependencies: one feature flag (`Win32_UI_Input`) on the existing `windows` 0.61
crate. No new crate, no network surface. The Raw Input target is a hidden window of
the built-in `STATIC` class, so no window class is registered.

## Alternatives considered

- **Map `AltRight` → `VK_RMENU` in `global-hotkey`.** Rejected: registers but never
  fires (silently dead).
- **`RegisterHotKey(MOD_ALT, VK_MENU)`.** Rejected: cannot tell left from right
  Alt, and it swallows Alt system-wide.
- **`WH_KEYBOARD_LL` hook that consumes Right-Alt.** Rejected: sees and can alter
  all input; Windows silently removes hooks that exceed `LowLevelHooksTimeout`;
  more likely to trip security heuristics; would also break AltGr typing unless it
  re-implemented chord logic anyway.
- **A different Windows default chord.** Rejected by the operator: breaks the
  "hold Right-Alt everywhere" identity.

## Consequences

- Windows first run can start with a working default hotkey.
- Masking uses `SendInput`, which UIPI blocks for elevated foreground apps; there the
  menu may still activate, but text injection into elevated apps is already
  blocked, so dictation there fails visibly either way.
- Must maintain: the listener never records key identities. Any change that makes
  key data leave the edge reducer needs a new ADR.
- macOS and Linux are unchanged; `Signal::Chord` is platform-agnostic, and only the
  Windows listener emits it today.
