<!-- Public copy for the Mac/Windows agents. -->
# Kaydence: Build vs Borrow (Linux dictation landscape, 2026-09-27)

*The question: are we reinventing wheels that tried-and-true open-source projects have already built?
Covered here:*
- *voxtype, installed on this machine;*
- *the "OSS" repo you heard about;*
- *the seven tools in the Weesper Neon Flow article;*
- *the closest cross-platform peers (Handy, OpenWhispr, Whispering/Epicenter);*
- *Omarchy-specific tools (hyprwhspr, OSTT);*
- *a scorecard against Kaydence's own original pro/con list (`docs/COMPETITIVE-ANALYSIS.md` §2–§3).*

*Method: four research agents cloned or read the repos and issue trackers. I verified the
decision-critical claims myself (commands and links below). Kaydence is **MIT**, so MIT/Apache code
can be copied with attribution; **GPL/AGPL code can only inform the design**.*

---

## 1. Bottom line

1. **The core isn't being reinvented.** The best Linux tools converged on the same architecture
   Kaydence already has: Rust, cpal, whisper-rs, and a compositor keybinding that calls a small CLI.
   voxtype (Omarchy's default), Handy and OSTT all look like this.
2. **Kaydence's safety features are genuinely unique.** No surveyed tool refuses password fields,
   stops typing when focus moves, or writes audio to disk before transcribing.
   - voxtype and Handy have **no** password or focus safety (voxtype #801, Handy #315 are open).
   - OpenWhispr *detects* password fields only for its assistant, not for dictation.
3. **Stop building these four things ourselves:**
   - **Parakeet and Silero lanes on raw ONNX Runtime.** `transcribe-cpp` (MIT, ships in Handy and Epicenter) and `earshot` (pure-Rust VAD, MIT/Apache) do this.
   - **GNOME/KDE typing.** Use `ashpd` (MIT, 15.7 M downloads) for the portal, plus libei.
   - **Clipboard fallback.** Handy's MIT `paste_tx` restore solves the race everyone else has.
   - **Model-download hardening.** Handy's MIT download manager has resume, a stall watchdog, a mirror and delete-on-mismatch.
4. **One finding changes an ADR test plan.** voxtype shipped the same fitted-window speed-up as
   ADR-0024 in January 2026. Users on **large-v3-turbo + GPU** got repeated-phrase loops, and voxtype
   switched it off by default (voxtype #124 → #130).
   - PR #58 already has the fixes voxtype landed on: `no_context`, a raised floor, a +128 margin.
   - The Mac/Windows handoff now **gates on GPU lanes and a large-v3-turbo loop check**.
   - Recommended hardening: round the window up to a multiple of 8, and retry at the full window when
     the output looks degenerate.
5. **The "OSS" repo is most likely OSTT** (github.com/kristoferlund/ostt), about 65 % confidence.
   Spoken aloud, "OSTT" sounds like "OSS-T". It is an Omarchy/Hyprland speech-to-text tool in Rust.
   Its author's own launch video calls it a "Text-to-Speech app", which explains the STT/TTS confusion.

## 2. The "OSS" repo: ranked candidates

| # | Candidate | What | Why it matches "OSS" | Confidence |
|---|---|---|---|---|
| 1 | **OSTT** · [kristoferlund/ostt](https://github.com/kristoferlund/ostt) | Terminal/popup STT for Omarchy, Rust + whisper-rs (CUDA/Vulkan) or BYOK cloud. Output goes to clipboard, stdout, file, AI or shell. 299★; last push 2026-09-24; topics include `omarchy`, `hyprland`, `wayland`. GitHub detects no license; an agent read MIT (*unverified*). | Sounds like "OSS-T". The launch video "OSTT – A visual speech-to-text popup for Omarchy…" calls it a "Text-to-Speech app". | ~65 % |
| 2 | **Open Speech Studio** · [OpenAEC-Foundation/open-speech-studio](https://github.com/OpenAEC-Foundation/open-speech-studio) | Tauri 2 + Rust + whisper-rs dictation, plus Piper TTS. Apache-2.0, 7★. | Calls itself "OSS" in its own code; does both STT and TTS. | ~12 % |
| 3 | **Handy** · [cjpais/Handy](https://github.com/cjpais/Handy) | The default "open-source" dictation app. MIT, 32.3k★. | "The OSS one" could have been heard as a name. | ~10 % |
| 4 | OpenMOSS "MOSS" (TTS / transcribe-diarize) | Open speech models | "MOSS" | ~6 % |
| 5 | openai/gpt-oss | An LLM, not speech; often part of local voice pipelines | "oss" | ~5 % |

**Check:** open [ostt.ai](https://ostt.ai) or the YouTube video. If it isn't OSTT, try #2, then #3.

**OSTT vs Kaydence:**
- Same core stack (Rust, whisper-rs, cpal). It is terminal- and clipboard-centric and doesn't type into the focused app.
- Its "re-run the same recording with another model" history feature is worth copying. Kaydence's
  write-ahead audio makes that nearly free.

## 3. Scorecard against Kaydence's original pro/con list (`COMPETITIVE-ANALYSIS.md`)

✓ = does it well · ◐ = partly · ✗ = no · — = not found or unknown.

| Original pattern | voxtype | Handy | OpenWhispr | OSTT | **Kaydence (now)** |
|---|---|---|---|---|---|
| **Praise-1** Edit tax gone (fillers, self-correction) | ◐ rules + filler list + LLM pipe | ◐ filler removal, custom words, LLM | ✓ LLM cleanup, dictionary, learns from edits | ◐ pipe to AI | ◐ Raw/Light dial; Full LLM is P2 |
| **Praise-2** Feels instant (≤ 1 s) | ◐ fitted window **off by default**; GPU/streaming options | ◐ June analysis: 2–5 s finalize; Parakeet faster (*unverified*) | — | — | ✓ **971 ms** live on laptop CPU; no streaming partials yet |
| **Praise-3** One hotkey, every app | ◐ Linux + macOS only | ◐ Wayland needs a CLI bind | ✓ auto-binds on Hyprland; GNOME portal | ◐ output via clipboard | ◐ Mac/Win native, Hyprland ✓; GNOME/KDE typing pending |
| **Praise-4** Technical vocabulary | ◐ replacements | ✓ fuzzy custom words | ✓ dictionary + auto-learn | — | ◐ dictionary + prompt hints |
| **Praise-5** Never loses a recording | ✗ in memory; lost on crash or VRAM failure (#723) | ◐ saved after the fact | — | ◐ keeps a history | ✓ **written to disk before ASR** |
| **Praise-6** Per-app context | — | — | ◐ terminal-aware paste | — | ◐ profiles from the frontmost app |
| **Complaint-1** Privacy overreach | ◐ local, but **logs full transcripts to the systemd journal** (seen on this machine) | ✓ | ◐ has its own paid cloud tier and account | ✓ | ✓ structural: no telemetry, network-audit gate |
| **Complaint-2** Resource hogging | ◐ Rust, 392 MB peak here | ✓ Tauri, 18–42 MB installers | ✗ Electron, **242–508 MB** installers | ✓ | ✓ ~147 MB idle with ASR; 8.56 MB .deb |
| **Complaint-6** Windows second-class | ✗ no Windows | ✓ | ✓ | ✗ | ✓ CI on all three OSes |
| **Complaint-7** Reliability edges | ◐ many typing scars (below) | ◐ Linux typing bugs #429/#439; clipboard race #502 | ◐ terminal/non-QWERTY paste bugs | — | ◐ P1 timing rules; 9/9 live typing; Chromium/Electron untested |
| **Complaint-8** Setup complexity | ✓ on Omarchy (installer, menu, bar) | ✓ | ✓ | ✓ | ◐ Omarchy snippet is manual; auto-bind planned (K-13) |
| **Password-field refusal** | ✗ | ✗ | ◐ detects, doesn't block | n/a | ✓ **proven live, 3/3** |
| **Stops if focus moves mid-typing** | ✗ (#801 open) | ✗ (#315 open) | ◐ restores focus on Windows | n/a | ✓ **proven live (0 stray keys)** |
| License | MIT | MIT | MIT | not detected | MIT |

**Verdict:** Kaydence still occupies the gap it set out to fill: local, intelligent, cross-platform,
lightweight, *and* safe. The one flank it hasn't covered yet is **Praise-1** (cleanup quality), which
is P2. OpenWhispr leads there.

## 4. voxtype (installed here): pros, cons, and what to take

**What it is:** MIT, Rust, 1,560★, one maintainer with very high churn (187 commits in 30 days).
Omarchy's official dictation tool.
- It has been running here since 09-18 as a user service, with 392 MB peak memory. Its model is `base.en` on CPU; a Vulkan variant is installed but not used.
- Binds: F9 push-to-talk and SUPER+CTRL+X toggle. Kaydence uses F10 and SUPER+ALT+D, so they coexist.

**Pros:**
- Engine breadth: whisper-rs, parakeet-rs, Moonshine, SenseVoice and more.
- GPU variants: Vulkan, CUDA, ROCm, NPU.
- Streaming and meeting mode.
- An official Omarchy integration: installer, bar widget via `status --follow` JSON, menu.
- Solid cleanup rules: filler words with a per-language collision list; spoken punctuation.

**Cons:**
- It shells out to wtype, ydotool and similar tools, and inherits their bugs:
  - first character dropped (#61);
  - CJK corruption (#695, open);
  - German y/z swapped (#120);
  - stuck modifiers (#538).
- No password or focus safety.
- Audio lives only in memory.
- Transcripts logged to the journal at INFO level.
- No Windows.

**Take from it (all MIT):**
1. Whisper-decode hardening: an 8-aligned `audio_ctx`, plus a retry at the full window on degenerate output.
2. Detect digital silence and dead streams; drop known hallucinations ("you", "Thank you.", "♪").
3. Its issue list as our regression suite: long CJK text, first-character drop, German layout, stuck modifiers.
4. Held-modifier protection through a Hyprland submap over IPC. This needs no `input` group and becomes **K-11**.
5. Omarchy polish: the `status --follow` JSON shape for the bar, a `cancel` verb, and avoiding voxtype's binds.

**Keep ours:**
- The socket CLI. Handy found that WebKitGTK uses SIGUSR1 internally (#1660), so never use signals.
- Write-ahead audio.
- In-process Unicode typing.
- Password and focus safety.

**For you (not a Kaydence task):** voxtype on this machine logs your dictations in full to
`journalctl --user -u voxtype`. It also keeps about 390 MB resident even if you only use Kaydence.
Stopping it is your call: `systemctl --user disable --now voxtype`.

## 5. The article's seven tools, plus notable extras

| Tool | License / ★ / activity | How it types on Wayland | Verdict for Kaydence |
|---|---|---|---|
| Vocalinux | **AGPL-3.0** (the article says GPL) / 884 / v0.17.0 Sep 2026 | **IBus engine `commit_text`**, then wtype/ydotool, then clipboard | Ideas only. Its IBus route gives full Unicode on GNOME without root, but has hard edges (#523, #574, #607). |
| VOXD | MIT / 298 / dormant since Oct 2025 | ydotool everywhere | Ignore |
| Handy | MIT / 32.3k / v0.9.7 Sep 2026 | Shells out to wtype, kwtype, dotool or ydotool; clipboard + chord | **Borrow heavily** (§6) |
| OpenWhispr | MIT / 8.7k / weekly releases; Electron | Paste chord via wtype, then **RemoteDesktop portal with a saved restore token**, then `hyprctl sendshortcut`, then uinput | Borrow patterns (portal token, Hyprland auto-bind, whisper thresholds) |
| nerd-dictation | GPL-3.0 / 1,925 / Oct 2025 | ydotool, dotool, wtype | Ideas only |
| Whispering / Epicenter | **AGPL** from v7.8.0; **MIT up to v7.7.2** / 4.8k | clipboard + enigo | Only the v7.7.2 code is copyable; macOS `ConcealedType` clipboard tagging is a public convention |
| LinuxWhispr | MIT / 7★ / one commit | — | Ignore (a scaffold) |
| *hyprwhspr* (extra) | MIT / 1,221 / v1.45 Sep 2026 | Clipboard + `hyprctl sendshortcut`, then wtype, then ydotool | Borrow: restore only if the clipboard is unchanged; check `hyprctl`'s reply (it exits 0 on errors, #218) |
| *Speech Note (dsnote)* | MPL-2.0 / 1,673 | ydotool; **GlobalShortcuts portal** for push-to-talk | Ideas: XKB compose trick; portal push-to-talk |
| *fcitx5-vinput* (extra) | GPL-3.0 / 473 | fcitx5 addon `commitString` | Ideas. Omarchy runs fcitx5 by default (running here). |

**The article's own bias:** it is published by Weesper Neon Flow (a Mac/Windows product), and its
license and version facts are partly outdated.

## 6. Build vs borrow, component by component

| Component | Kaydence today | Proven option | License | Verdict → where |
|---|---|---|---|---|
| Parakeet lane | ADR-0016 plan: raw ONNX Runtime | **`transcribe-cpp` 0.2.4** (ggml; Whisper + Parakeet; Metal/Vulkan; used by Handy and Epicenter) · `transcribe-rs` 0.3 · `parakeet-rs` | MIT | **ADOPT, pending evaluation** → ADR-0016 amendment. Caveat: Parakeet models are 456–740 MB, over the 250 MB resident budget, so opt-in quality lane only. It also avoids ORT's AVX2 crash on pre-Haswell CPUs. |
| VAD | Energy gate; Silero-via-ORT planned | **`earshot` 1.2.2**: pure Rust, no model file, 241k downloads | MIT/Apache | **ADOPT** → ADR-0016 amendment |
| Whisper decode | Fitted window (ADR-0024) | voxtype: 8-alignment + degenerate retry. OpenWhispr: `entropy_thold 2.8`, `logprob_thold −1.25` (they report hallucinated tails 2.25 % → 0.06 % over 4,814 dictations) | MIT | **BORROW** → ADR-0024 follow-up, measured |
| GNOME/KDE typing | Pending (K-19) | **`ashpd`** (RemoteDesktop + GlobalShortcuts portals) + libei via `reis` (MIT) or `eitype` (Apache-2.0). Keep one session and save the restore token. | MIT/Apache | **ADOPT** → K-19. Characters outside the layout go via clipboard until libei `ei_text` ships in Mutter (still a draft). |
| Clipboard fallback | Planned (restore ≤ 200 ms) | **Handy `paste_tx`**: restore only after the target has read the clipboard; skip if it changed. Mark it sensitive (`x-kde-passwordManagerHint`, `ConcealedType`); explicit `text/plain` MIME. | MIT | **BORROW** → inject/ |
| Model download | ADR-0017 downloader | Handy `download.rs`: Range resume, 60 s stall watchdog, mirror, delete on mismatch | MIT | **BORROW** the resume and watchdog → models/ |
| Held modifiers | K-11 open | voxtype's Hyprland submap during output, driven over IPC | MIT | **BORROW** → K-11 |
| Omarchy integration | Snippet file | voxtype `status --follow` JSON and `cancel`; OpenWhispr consent-style `hyprctl keyword bindt/bindrt` auto-bind | MIT | **BORROW** → K-13, K-15 |
| Cleanup | Light rules | Handy `text.rs` fuzzy custom words; voxtype filler collision list | MIT | **BORROW** → cleanup/, dictionary/ |
| Wayland typing (Hyprland) | In-process virtual keyboard, per-dictation keymap | wtype / ydotool (what voxtype and Handy shell out to) | — | **KEEP OURS.** Theirs inherits the bugs listed above. |
| Hotkey IPC | 0600 socket + `kaydence-ctl` | Signals (voxtype); CLI flag or SIGUSR2 (Handy) | — | **KEEP OURS** |
| Password / focus safety, write-ahead audio | Built and proven | None found in any surveyed tool | — | **KEEP OURS** (a differentiator) |
| Optional future | — | IBus (GNOME) or fcitx5 (Omarchy, KDE) input-method bridge, e.g. `fcitx5-text-bridge` (MIT) | MIT / ideas | **Explore** (§7) |

**One agent claim, corrected:** an agent said Kaydence's virtual keyboard has wtype's Chromium bug,
where the 14th character lands on keycode 22 and acts as BackSpace (wtype #71). **It doesn't.** Kaydence
only uses 48 printable-position keycodes (`xkb.rs::SAFE_EVDEV_CODES`) and never 9, 22, 23 or 36
(Esc, BackSpace, Tab, Return). That came from the F1 fix. The gap that remains is a **Chromium/Electron
live test** (Omarchy's browser is Chromium), which is now on the list.

## 7. What works where on Linux in 2026 (typing text)

| Method | GNOME | KDE | Hyprland / wlroots | Unicode | Cost |
|---|---|---|---|---|---|
| Virtual keyboard (Kaydence, wtype) | ✗ | ✗ | ✓ | Full (per-dictation keymap) | none |
| RemoteDesktop portal + libei | ✓ | ✓ | ✗ (xdph #402 open) | Active layout only, until `ei_text` ships | one consent dialog |
| IBus engine commit | ✓ | only with KWin VK set to IBus | only with ibus-wayland | Full | engine switching |
| fcitx5 addon commit | if fcitx5 is the input method | if fcitx5 is the input method | ✓ (**Omarchy default**) | Full | a system addon |
| uinput (ydotool/dotool) | ✓ (flaky chords) | ✓ | ✓ | Layout keycodes only | root/udev (on this box `/dev/uinput` is root-only) |
| Clipboard + paste chord | ✓ via libei/uinput | ✓ | ✓ via `hyprctl sendshortcut` | Full | clipboard exposure |

**Hotkeys:** every mature tool converged on **compositor or desktop binds calling a CLI**, which is
Kaydence's approach. The GlobalShortcuts portal (GNOME 48+, KDE 5.27+) gives push-to-talk
press/release on GNOME and KDE; use `ashpd` for it.

## 8. What this changes

- **ADR-0023 (#57):** strengthened. It matches the field's consensus design, and Kaydence alone has
  in-process Unicode typing plus password/focus safety.
- **ADR-0024 (#58):** the technique is used in the field (voxtype), but voxtype's loop reports make
  the **GPU and large-v3-turbo checks mandatory** before acceptance (the handoff covers this). Adding the
  8-alignment and degenerate retry first is cheap insurance: about an hour including re-proof. Say
  the word.
- **ADR-0016 (Parakeet/Silero on ONNX Runtime):** worth revisiting. `transcribe-cpp` plus `earshot` could replace the planned ORT dependency. This is an operator decision.
- **New tests to add:**
  - Chromium and Electron live typing;
  - a long CJK passage with fcitx5 plus a CJK engine;
  - voxtype's issue cases (first-character drop, German layout).
- **New tasks:**
  - K-11 via a Hyprland submap;
  - K-19 via `ashpd` + libei;
  - clipboard fallback via the `paste_tx` pattern;
  - K-25 quality lane via `transcribe-cpp`.

## Sources

**Repositories:**
- [peteonrails/voxtype](https://github.com/peteonrails/voxtype), issues #124, #130, #695, #801, #723
- [cjpais/Handy](https://github.com/cjpais/Handy), issues #1660, #502, #315
- [OpenWhispr/openwhispr](https://github.com/OpenWhispr/openwhispr), #1458
- [EpicenterHQ/epicenter](https://github.com/EpicenterHQ/epicenter)
- [kristoferlund/ostt](https://github.com/kristoferlund/ostt)
- [goodroot/hyprwhspr](https://github.com/goodroot/hyprwhspr)
- [VocaHQ/vocalinux](https://github.com/VocaHQ/vocalinux)
- [mkiol/dsnote](https://github.com/mkiol/dsnote)
- [ideasman42/nerd-dictation](https://github.com/ideasman42/nerd-dictation)
- [jakovius/voxd](https://github.com/jakovius/voxd)
- [xifan2333/fcitx5-vinput](https://github.com/xifan2333/fcitx5-vinput)

**Crates (crates.io, verified 2026-09-27):**
- `transcribe-cpp` 0.2.4 (MIT)
- `earshot` 1.2.2 (MIT/Apache)
- `transcribe-rs` 0.3.11 (MIT)
- `ashpd` 0.13.13 (MIT)

**Other:**
- [Weesper Neon Flow article](https://weesperneonflow.ai/en/blog/2026-06-18-voice-dictation-linux-open-source-tools-2026/)
- Local: `pacman -Qi voxtype-bin`, `/usr/share/omarchy/default/hypr/bindings/voxtype.lua`, `systemctl --user status voxtype`, `pgrep fcitx5`
