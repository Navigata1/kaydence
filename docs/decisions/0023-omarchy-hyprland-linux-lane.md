# ADR-0023: Omarchy/Hyprland Linux lane — rootless Wayland injection, compositor-bound hotkey, compositor focus truth

- **Status:** Proposed <!-- Human gate: adds dependencies (wayland-client, wayland-protocols-misc, rustix as a direct dep), a new local IPC surface (the control socket), and platform code. Listed in scripts/check-adr-status.sh PROPOSED_OK until the operator decides. -->
- **Date:** 2026-09-25
- **PRD items affected:** P0-3 (universal injection — Linux/Wayland lane, Unicode),
  P0-1 (hotkey — Wayland), Pitfall P9 (focus binding), Pitfall P10 (setup), ADR-0011
  (Linux first-class), ADR-0013 ladder rung 2 (`zwp_virtual_keyboard_v1`)

## Context

The operator made **Omarchy** (Arch Linux + Hyprland, Lua-configured since Omarchy 4)
the most important Linux target on 2026-09-25, with Ubuntu and Debian kept as the
packaging baselines. The first native x86_64 run on Omarchy 4.0.4 / Hyprland 0.56.2
(every existing CI gate green) exposed five facts the Debian ARM64 GNOME VM never could:

1. **The shipped app could not type on Linux.** `inject::platform_injector()` returned
   `UnimplementedInjector` on Linux; the validated AT-SPI + uinput path lived only in
   the `atspi-selftest` binary.
2. **Even a working injector would never fire.** The Linux frontmost-app detector was
   `UnknownFrontmostAppDetector`, and `verify_focus_binding` holds any delivery whose
   capture-time or delivery-time target is unverified — so every Linux dictation ended
   `Held{FocusChanged}`.
3. **The validated uinput path is unavailable on stock Omarchy.** `/dev/uinput` is
   `crw------- root root`; using it requires a udev rule, an `input` group change, and a
   re-login (sudo).
4. **The hotkey registers but is dead.** `tauri-plugin-global-shortcut` grabs keys over
   X11; inside a Wayland session that grab lands on XWayland and only fires while an
   XWayland window has focus. First-run reported "registered" — hotkeys invariant 3
   forbids that silent death.
5. **Omarchy already defines the dictation contract.** Its
   `default/hypr/bindings/voxtype.lua` binds F9 press → `voxtype record start`,
   F9 release → `voxtype record stop`, SUPER+CTRL+X → toggle. voxtype 1.0.1 is the
   incumbent on this platform.

## Decision

**Make Kaydence a native Hyprland/wlroots citizen: type through the compositor's
virtual-keyboard protocol with per-injection Unicode keymaps, take focus truth from
the compositor, and receive the hotkey as a compositor-run command.**

1. **Keystroke channel — `zwp_virtual_keyboard_v1` first (`inject/wayland_vk.rs`).**
   Rootless and compositor-mediated. Each injection uploads its own XKB keymap
   (`inject/xkb.rs`) in which every distinct character is one key at level 1, so emoji,
   CJK, RTL and combining marks type exactly, independent of the user's layout — closing
   the "ASCII only" gap on Linux (inject invariant 5). Keycodes stay ≤ 255 so XWayland
   clients work; longer passages roll to a fresh keymap. The keymap (which reveals the
   characters typed) lives in a sealed anonymous `memfd`, never a path. `/dev/uinput`
   remains the fallback **only when already writable**; Kaydence never escalates.
   AT-SPI native insert stays detection-only (ADR-0013 amendment). No clipboard
   fallback on Linux yet — capabilities say so.
2. **Focus truth — Hyprland IPC (`inject/hyprland.rs`).** One request
   (`j/activewindow`, 40 ms ceiling) on the compositor's per-user socket gives the
   frontmost app (window class → `profiles/`) and the focus-binding proof (P9), plus
   the window's pid. The in-app AT-SPI focus tracker's verdict (Editable / Secure /
   NoTarget) is trusted **only when its process matches the compositor-focused
   window's pid**; a stale observation from another app becomes `Unknown`. Without
   compositor truth (GNOME/KDE) only a `Secure` observation is honoured (fail-safe);
   everything else follows ADR-0013's disclosed Lenient/Strict policy.
3. **Hotkey on Wayland — `<app> record start|stop|toggle|status`
   (`hotkeys/control.rs`).** A compositor keybinding runs the CLI; the CLI sends one verb
   to the running app over `$XDG_RUNTIME_DIR/<app>/control.sock` (0700 directory,
   0600 socket, `SO_PEERCRED` same-uid check, one verb in / one state word out, never
   audio or transcript text). The app translates the verb into the **same Press/Release
   edges** the native grab produces (`hotkeys::control_signal`), so debounce, the
   250 ms floor and the 300 ms tail apply unchanged. Requests are served in order on
   one thread (P1's serialized owner). Socket-started captures carry a 5-minute
   safety stop (a compositor can lose a release). First-run status is honest: on
   Wayland the X11 grab no longer counts as "registered" unless a compositor binding
   is detected (`hyprctl binds` — arg for hyprlang binds, description for Lua binds);
   the first command received flips it to registered.
4. **Omarchy integration kit.** `apps/desktop/linux/omarchy/kaydence-bindings.lua`
   mirrors Omarchy's voxtype pattern: hold **F10** push-to-talk, **SUPER+ALT+D**
   toggle — both verified free in Omarchy 4.0.4 defaults, leaving voxtype's F9 and
   SUPER+CTRL+X untouched. It ships inside the `.deb`/`.rpm` and the Arch PKGBUILD
   (`apps/desktop/linux/arch/PKGBUILD`).
5. **CI.** An Arch Linux container leg (Omarchy's base distro) runs fmt, clippy and the
   test suite beside the existing Ubuntu/Windows/macOS matrix.

**Dependencies** (`cfg(target_os = "linux")` only; lockfile change is additive,
+7 packages, 0 changed): `wayland-client` 0.31 and `wayland-protocols-misc` 0.3 (MIT,
Smithay; both pre-approved by ADR-0013, pure-Rust wire protocol), and `rustix` 1
(Apache-2.0/MIT, already in the lockfile transitively; now direct for `memfd_create`,
`SO_PEERCRED`, `getuid`, `access`). **No network surface:** Wayland, Hyprland IPC,
AT-SPI and the control socket are all local Unix sockets; `audit-network.sh` passes.

## Alternatives considered

- **Shell out to `wtype`/`ydotool`.** A hidden runtime dependency, a process per
  injection, and opaque failures. Rejected.
- **Install a udev rule and make uinput primary.** Needs sudo + re-login, bypasses the
  compositor's input model, ASCII-only today. Kept as an opt-in fallback.
- **xdg-desktop-portal GlobalShortcuts.** `xdg-desktop-portal-hyprland` supports it,
  but the user must still add a `global` bind by hand, release semantics vary by portal
  backend, and it adds a D-Bus session flow. The control socket works identically on
  Hyprland, Sway, and GNOME/KDE custom shortcuts. The portal stays the plan for
  sandboxed (Flatpak) builds.
- **Auto-register binds at runtime through Hyprland IPC.** Zero-config, but it silently
  mutates the user's compositor and can collide with their binds. Rejected for an
  explicit snippet; a consent-gated "Add Omarchy keybinding" action is a follow-up
  (P10).
- **RemoteDesktop portal + libei.** The rootless answer for GNOME/KDE; out of scope for
  this Omarchy-first lane and still tracked under ADR-0013.

## Consequences

- **Easier:** Omarchy/Hyprland/Sway users get rootless, Unicode-complete injection
  with real focus binding; Linux gets its first frontmost-app detector; compositor
  keybindings, status bars (`record status`) and launchers can drive dictation.
- **Harder / limits (disclosed):**
  - GNOME/KDE Wayland offer no virtual-keyboard protocol and no focus API: delivery
    holds with "insert here / copy" unless uinput was opted into. The portal + libei
    rung remains the answer there.
  - Hyprland ORs modifier state across keyboards. Typing happens after release +
    tail + ASR, and the F10 bind has no modifiers; a still-held modifier from a custom
    modified binding could combine with typed keys. Follow-up: consult
    `hl.is_key_down` before the first key.
  - Hyprland's `ecosystem:enforce_permissions` can deny virtual keyboards; that
    surfaces as a protocol error → no keystroke channel, never a silent success.
  - Within one app, AT-SPI staleness is mitigated by dropping the observation on
    focus loss; apps that expose nothing remain `Unknown`.
  - Unit tests now build the hotkey runtime with inert OS seams (the real Linux
    injector would otherwise type into the developer's focused window).
- **Must maintain:** Hyprland's IPC and bind-JSON shapes (`hyprland.rs` tests pin the
  0.56 formats); the Omarchy snippet against Omarchy default bindings; the
  `wayland-selftest` + `scripts/linux-wayland-inject-proof.sh` harness.

## Amendment 2026-09-26 — AppImage installer budget (operator decision)

CI's first complete installers run measured the AppImage at **86,608,376 bytes**. It
bundles WebKitGTK/GTK from its Ubuntu 22.04 build host, while the .deb/.rpm (8.56 MB)
link the distro's own copy. The operator decided in session: *"the limit can be upped
if it creates a better program and smoothness and enjoyment."*

- **Decision:** the Linux **AppImage may reach 100 MB**. Native packages (.deb, .rpm,
  AUR/PKGBUILD) stay **< 60 MB**. `scripts/check-linux-packages.sh` enforces both
  limits strictly in CI. Non-negotiable #7 in the root `AGENTS.md` now names this one
  exception.
- **What it buys:** a one-file install that runs on distros with no native Kaydence
  package, whatever WebKitGTK they ship (or none). It does not make Kaydence itself
  faster. On Omarchy the native package, which uses the newer system WebKitGTK,
  remains the recommended install.

## Validation

Evidence: `ops/mission/evidence/2026-09-25-omarchy-linux-lane.txt` (Omarchy 4.0.4,
Hyprland 0.56.2, x86_64). It records every gate result, the live Hyprland IPC and
virtual-keyboard probes, the control-socket round trip, and — honestly marked — any
live proof still pending an unlocked interactive session.
