<!-- Public copy for the Mac/Windows agents. Operator-private notes (other local tools, security settings) are omitted. Canonical: the operator's ~/Work copy. -->
# Kaydence on Linux: State of the Union & Plan of Attack

*Flagship report, 2026-09-25 · updated 2026-09-26 (fast-network retry) · **updated again
2026-09-26 evening** after the operator's four priorities. Platform priority: **Omarchy (Arch +
Hyprland) first**; Ubuntu and Debian are the packaging baselines. Author: Claude (Claude Code) for
the operator. Companions: the Mac/Windows verification handoff, the build-vs-borrow study, PR #57 (Linux lane),
PR #58 (ASR speed), and the two evidence files.*

---

## 1. Bottom line

**Kaydence is now a working Omarchy dictation app on this laptop.** You hold the key, speak, and
the text lands in the focused window **about one second after you let go**:
- **971 ms** for 4.9 s of speech, down from 4,055 ms.
- **~1.1 s** for 11 s of speech, down from ~5.9 s.

These are live runs of the real app, with a virtual mic, typing into a real window.

All four of tonight's priorities are done and proven live:

| # | Priority | Result | Where |
|---|---|---|---|
| 1 | Password field focused *before* Kaydence was tracking (#8) | **Fixed.** That exact case now ends `Held{SecureField}` with the field empty; it typed into the field before the fix. | PR #57 `9cac9aa` |
| 2 | ASR speed on x86 CPU | **Fixed.** One whisper pass per dictation plus a fitted encoder window. Bench p95 4,062 → **742 ms**; live key release → text **971 ms**. | PR #58 `acddbdd` |
| 3 | AppImage over the 60 MB budget | **Decided (your call):** AppImage ≤ 100 MB; .deb/.rpm/AUR stay < 60 MB. Both limits enforced strictly in CI. | PR #57, ADR-0023 amendment |
| 4 | Keys lost when focus moves mid-delivery (P9) | **Fixed.** Every 4 keys are round-tripped and focus is re-checked. A focus steal left the new window with **0** stray characters. | PR #57 `9cac9aa` |

Also done tonight, on your instruction:
- `cmake` and `patchelf` installed through your sudo prompt.
- Microphone unmuted.

**CI:** PR #57 is **7/7 green** at `9cac9aa`; PR #58 is **4/4 green** at `acddbdd` (macOS, Windows, Ubuntu; run 36293039124).

**What the live test caught.** The first speed fix (a 384-frame window floor) passed the bench, but
the live end-to-end run typed "**As not**". The app's voice detector sends each pause-separated
fragment to whisper on its own, and a lone 1.47 s fragment garbles in a narrow window. The final
design sends the whole dictation as one pass. It is both faster and more accurate. The bench alone
would have shipped the regression.

## 2. Where Kaydence stands overall (mission ledger)

| Phase | Status | Linux note |
|---|---|---|
| P0 Toolchain & law | ✅ done | 3-OS CI matrix plus Arch and Debian legs and an installers job |
| P1 MVP dictation | 🟡 in progress (G3 latency, G4 UI) | Linux delivery (PR #57) and the x86 latency fix (PR #58) both exist; both need your ADR decisions |
| P2 Cleanup differentiator | ⏳ pending | platform-neutral |
| P3 Daily-driver + Whisper-Ahead | ⏳ pending | HUD overlay needs Wayland layer-shell work |
| P4 Flagship trio | ⏳ pending | Relay's mixed fleet includes Linux |

## 3. State of the Union: Linux

### 3.1 Scorecard

| Capability | Before (main `f5f4eb6`) | Now | Evidence |
|---|---|---|---|
| Text delivery in the app | ❌ `UnimplementedInjector` | ✅ `LinuxTextInjector` | #57 |
| Unicode typing (live) | ❌ ASCII-only uinput | ✅ **8/8 byte-exact**: CJK, RTL, emoji, accents, multi-keymap | §10.1 |
| Root needed to type | ⚠️ yes | ✅ no (Hyprland/wlroots) | §5.2 |
| Wrong window at delivery time (P9) | ❌ every delivery held | ✅ Hyprland IPC | #57 |
| **Focus change *during* typing (P9)** | ❌ no check | ✅ **re-checked every 4 keys; new window got 0 chars** | §10.12 (b) |
| Password-field refusal (#8) | selftest only | ✅ **live, including a field focused before tracking** | §10.11, §10.12 (a) |
| Global hotkey | ❌ unregistrable (`AltRight`) | ✅ compositor bind → `kaydence-ctl` (2.1 ms) | §5.3 |
| Hotkey → mic running | unmeasured | ✅ ≈26–30 ms (derived; device open 23 ms measured) | §10.7 |
| Real speech → text (ASR) | ❌ never on x86 | ✅ JFK 11 s; RTF **0.066** | #58 |
| **Release → text, bench p95, x86 CPU** | 4,062 ms ❌ | **742 / 636 ms** ✅ (budget 1,200) | #58 §6 |
| **Release → first typed char, live** | 4,055 ms | **971 ms** (4.9 s speech) · **1,069–1,108 ms** (11 s) | #58 §6 |
| Idle RAM / CPU with ASR | 107.6 MB / 0 % | ~147 MB / 0 % (≤ 250 MB / ≤ 1 %): the decoding state now stays resident | #58 §6 |
| Installers | generic, shipped a selftest | ✅ deb/rpm **8.56 MB**, `kaydence` + `kaydence-ctl` only, gated | §10.8, §10.10 |
| AppImage | none | ✅ **86.6 MB within the new 100 MB AppImage limit** (your decision) | ADR-0023 amendment |
| CI | Ubuntu only for Linux | ✅ **7/7 green at `9cac9aa`**, strict on both size limits | run 36289977094 |

### 3.2 Why this lane is strong
1. **It uses the compositor instead of fighting it.** No root, no udev, no daemon.
2. **Typing doesn't depend on keyboard layout.** Each dictation carries its own tiny keymap, so any
   script types exactly, proven live.
3. **Focus truth, before and during typing.** Kaydence knows which window your words are going to.
   It stops mid-sentence if you switch, and the rest waits in history.
4. **A password field is refused even if it had focus before Kaydence started watching.**
5. **Speed without accuracy loss.** One pass per dictation reads the sentence whole. That is 5×
   faster and gives better punctuation than per-pause fragments.
6. **Evidence discipline.** Tonight the live E2E caught a regression that the bench missed; it was
   recorded, root-caused and fixed, not hidden.

### 3.3 Findings
Fixed tonight:
- **F7 ✅ x86 CPU ASR was 4–6× slower than it needed to be.** Two causes: a fixed 30 s encoder
  window per pass, **and** one pass per pause. Both are fixed in PR #58.
- **F12 ✅ Focus-before-tracking gap (#8).** Fixed with a bounded on-demand lookup of the focused
  accessible: at most 3,000 nodes and 150 ms, on a helper thread. A timeout means Unknown, as
  before.
- **F14 ✅ New: fragments garble in narrow windows.** The voice detector closes a segment after
  300 ms of silence. A lone 1.47 s "ask not" read "S not." at 384 frames and "as not." at 448–512.
  It is correct from 576 up; the floor is now 640. One pass per dictation makes this rare anyway.
- **F15 ✅ New: flushing is not delivering.** The first P9 guard only flushed each batch, and 12
  queued keys followed focus to the new window. Each batch is now round-tripped before the focus
  check, and the new window received 0.

Still open:
- **F13 🟡 GTK4 drops in-flight keys on focus-out.** Mid-delivery this is now covered by the P9
  guard. A client that loses focus within milliseconds *after* the last key can still drop keys.
  This was seen only when a test harness yanked focus, never in normal use. Optional follow-up: a
  ≤ 100 ms post-delivery settle.
- **F16 🟡 Model edge on one word.** On the 11 s JFK clip the full sentence reads "Americans
  **asked** not". The unchanged adapter reads it the same way with the full 30 s window. Per-pause
  fragments said "Ask not!" only because they heard the two words alone. The fix is a bigger model,
  not a wider window (see L1).

Unchanged from before:
- **F1–F4:** physical-keymap binds, keymap forwarding, Lua dispatch, exec cost.
- **F5 (correction):** the mic opens in ~23 ms.
- **F6:** `RightAlt` can't be registered on X11.
- **F8:** the Tauri selftest bug, fixed and gated.
- **F9:** AppImages are built on Ubuntu 22.04.
- **F10:** the lock screen holds keyboard focus.
- **F11:** the AppImage is 86.6 MB. Now decided.

### 3.4 Risks

| # | Risk | Severity | Mitigation / task |
|---|---|---|---|
| R1 | ADR-0024 (ASR) changes behaviour on every platform | Medium | Golden/WER run on macOS (Metal + CPU) and Windows (ARM64 + x64) before Accepted |
| R3 | A held modifier plus virtual keys fires the user's binds | High | K-11 modifier guard |
| R4 | GNOME/KDE have no virtual-keyboard protocol or focus API | Medium | K-19 portal + libei |
| R5 | #56 conflicts; ADR-0023/0024 await you | Low | trivial rebase; your decisions |
| R6 | Packaged builds use AVX2 via ggml defaults; pre-Haswell CPUs could crash | Medium | K-21 runtime CPU dispatch |
| R8 | base.en misses some words at liaisons ("asked not") | Low–Medium | K-25: offer small.en / large-v3-turbo quality lane on Linux |
| ~~R1 old~~ | ~~x86 p95 4 s~~ | fixed | #58 |
| ~~R2~~ | ~~field focused before launch~~ | fixed | #57 |
| ~~R7~~ | ~~AppImage over budget~~ | decided | ADR-0023 amendment |

## 4. Plan of Attack: Linux

### L0: Land the lane and the speed fix
- ✅ Live typing 8/8 plus focus-steal (9/9).
- ✅ Password field refused, 3/3 cases.
- ✅ Live E2E passing.
- ✅ CI 7/7 on `9cac9aa`.
- ✅ AppImage decided.
- ✅ PR #58 CI 4/4 green on `acddbdd`.
- 🛑 **Your decisions: ADR-0023 (#57) and ADR-0024 (#58).**
- Rebase after #56 (one line in `check-adr-status.sh`).

**Exit:** both PRs green, then your go.

### L1: Omarchy daily driver (1–2 weeks) → *"hold F10, talk, clean text lands in < 1.2 s"*
Reached on this laptop tonight.
1. **K-9** Extend the E2E to ≥ 20 runs and 3 real apps: a browser, a code editor, and chat.
2. **K-11** Modifier guard.
3. **K-25** Linux quality lane: small.en or large-v3-turbo, as an opt-in, with the bench and footprint table.
4. **K-13** First-run Linux rows, plus a consent-gated "Add Omarchy keybinding".
5. **K-15** Tray and Omarchy shell status widget.
6. **K-14** Linux default hotkey.
7. **Streaming follow-up:** transcribe segments during capture, so even long dictations finish at ~0.6 s.

**Exit:**
- ✅ x86 p95 ≤ 1.2 s.
- ✅ Password field refused live.
- 20 logged dictations.
- First dictation ≤ 60 s after install.

### L2: Distribution (weeks 2–4)
- CI installers → GitHub releases: deb/rpm < 60 MB and an AppImage ≤ 100 MB.
- 🛑 AUR `kaydence-git`. Its build tools are now installed here.
- 🛑 Upstream to Omarchy: `bindings/kaydence.lua` and an Install › AI entry.
- Ubuntu 24.04 and Debian 12 VM validation.
- K-21 CPU-dispatch builds.

### L3: Breadth (month 2)
- Sway focus.
- GNOME/KDE portal + libei.
- X11 XTest.
- Clipboard fallback with a ≤ 200 ms restore.

### L4: Linux flagship
- Whisper-Ahead layer-shell HUD.
- Relay across a Mac + Windows + Omarchy fleet.

## 5. Scoreboard

| KPI | 09-25 | 09-26 day | **09-26 evening** | L1 target |
|---|---|---|---|---|
| Live Unicode typing cases | 6/7 | 8/8 | **9/9** (incl. focus-steal) | + 3 real apps |
| CI jobs green | pending | 7/7 | **7/7 at `9cac9aa`** | all |
| Release → text p95, x86 bench | unmeasured | 4,062 ms | **742 / 636 ms** | ≤ 1,200 ✅ |
| Live release → first typed char | — | 4,055 ms | **971 ms** | ≤ 1,200 ✅ |
| Idle RAM with ASR | — | 107.6 MB | **~147 MB** | ≤ 250 MB |
| Password-field proof cases | — | 2/2 | **3/3** (incl. focused before tracking) | ✅ |
| Keys leaked to a new window on focus steal | untested | untested | **0** | 0 ✅ |
| Installer size (deb, with ASR) | 7.7 MB | 8.56 MB | 8.56 MB | < 60 MB |
| AppImage size | — | 86.6 MB (over) | **86.6 MB (≤ 100 MB limit)** | ✅ |

## 6. What only the operator can do
1. **Decide ADR-0023 (#57) and ADR-0024 (#58).** Both are Proposed by design. Approve the fork CI
   when GitHub asks.
2. Before ADR-0024 moves to Accepted, run the golden/WER check on your Mac and Windows machines.
   The handoff has the commands.
3. Done tonight: ~~unmute the mic~~ · ~~install cmake/patchelf~~ · ~~AppImage decision~~ ·
   ~~unlock for live proofs~~.
