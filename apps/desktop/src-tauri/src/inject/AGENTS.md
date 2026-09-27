# inject/ — Text Delivery (highest platform risk in the repo)

## Owns
Getting final text into the focused control: macOS Accessibility API insertion
(CGEvent fallback), Windows UI Automation ValuePattern/TextPattern insertion
(SendInput fallback), the shared clipboard fallback (snapshot → set → paste →
restore ≤200 ms), secure-field detection/refusal, focus-binding.

## Does not own
Choosing the text (upstream), app identification for profiles (profiles/ —
though we share the frontmost-window plumbing via a common helper).

## Invariants
1. **Method ladder per OS:** native insertion → synthesized keystrokes →
   clipboard fallback. Record which method succeeded in the `Injected` event;
   per-app method overrides live in profiles when an app is known-broken.
2. **Clipboard fallback never destroys user data:** snapshot all formats we
   can, restore within 200 ms, keep the snapshot until restore is confirmed.
   (Pitfall P2 — this is a top-3 category complaint.)
3. **Secure fields:** if the focused control is a password/secure input,
   refuse, emit `Held{reason: SecureField}`, notify via HUD. Never type into
   it, never store what would have gone there beyond normal history.
4. **Focus binding:** the injection target is captured at session start. If
   focus changed by delivery time, do NOT inject — emit `Held{reason:
   FocusChanged}`, park the text in the HUD with one-tap "insert here /
   copy". (Pitfall P9.)
5. Unicode-complete: emoji, CJK, RTL, combining marks. Keystroke synthesis
   paths must handle layout independence (use unicode injection, not VK codes,
   wherever the OS allows).

## The app-compat matrix
`tests/integration/inject_matrix.md` tracks the top-20 target apps per OS
(browsers ×3, VS Code/Cursor, terminals, Slack/Discord, Office/Google Docs,
Mail, Notion, Obsidian…). Every release verifies the matrix; a regression on a
matrix app blocks release. Electron apps and web editors are the usual
offenders — expect per-app quirks and document each workaround inline.

Known per-app quirks (documented, not defects):
- **Windows — the new WinUI/RichEdit Notepad** coalesces rapid synthetic
  `KEYEVENTF_UNICODE` events, so the *keystroke fallback* rung can drop repeated
  characters there. It is a non-issue in practice: Notepad exposes a writable UIA
  `ValuePattern`, so the native-primary path handles it cleanly. Validated
  2026-07-09 (native insert PASS; keystroke PASS on classic Win32 Edit).
- **Linux GNOME/Wayland:** AT-SPI-gated uinput is live-proven for an accessible
  `Text` field and refuses a real `PasswordText` field. The current uinput
  keymap is US-QWERTY ASCII only, so the uinput rung refuses (holds) any text it
  cannot type completely rather than deliver a partial sentence; optional
  portal/libei remains a required follow-up. Opaque clients still follow
  ADR-0013's disclosed Lenient/Strict policy boundary.
- **Linux Hyprland/Omarchy + wlroots (ADR-0023):** the shipped backend is
  `linux.rs::LinuxTextInjector`. Keystrokes go through
  `zwp_virtual_keyboard_v1` (`wayland_vk.rs`) with a per-injection keymap
  (`xkb.rs`): rootless and Unicode-complete, keycodes ≤ 255 for XWayland
  clients. Focus truth comes from Hyprland IPC (`hyprland.rs`, the shared
  frontmost-window helper for `profiles/`). The AT-SPI verdict is trusted only
  when its pid matches the compositor-focused window. When no event vouches for
  that pid (a field focused before launch, a missed event), delivery runs a
  bounded on-demand lookup of the focused accessible in that process (≤ 3,000
  nodes, ≤ 150 ms, helper thread; a timeout means Unknown). During typing the
  target window is re-checked every 4 keys, each batch round-tripped first, and
  the rest is withheld if focus moved (P9 during delivery). Proof harnesses:
  `scripts/linux-wayland-inject-proof.sh` (incl. focus-steal) and
  `scripts/linux-secure-field-proof.sh` (incl. a password field focused before
  launch).
- **Never call `platform_injector()` from a unit test.** On a Linux desktop it is
  real and types into the developer's focused window; tests use fakes or
  `UnimplementedInjector`.

## Definition of done
All three OSes, matrix green, clipboard-restore test green, secure-field test
green, focus-change test green.
