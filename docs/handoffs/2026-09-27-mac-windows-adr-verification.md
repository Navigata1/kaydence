# Kaydence: Mac + Windows verification handoff (ADR-0023 / ADR-0024)

*2026-09-27 · from Claude (Omarchy, Linux x86) · for the agent or operator on the **Mac (Apple Silicon)** and
**Windows (ARM64 UTM VM, plus an x64 PC if available)**. Budget: about an hour per machine, mostly compile time.*

## Corrections, 2026-09-27 (read first; these override the sections below)

1. **Machines.** Windows is a physical **x64 NUC** (the "x64 PC" below). The **Windows ARM64** lane
   stays with the Mac's UTM VM.
2. **New #58 head: `bce00f4`, CI 4/4 green.**
   - It adds the approved safeguards: the fitted window is rounded up to a multiple of 8, and a
     degenerate pass is re-run with the full 1500-frame window (`whisper::degenerate_reason`).
   - Run T1–T4 against **`bce00f4`**, not `acddbdd`:
     `git fetch origin pull/58/head:pr-58 --force`.
   - T1/T4: also note whether the log line `looked degenerate … re-running with the full window`
     appears. That line means the fallback fired.
3. **#57 stays frozen at `9cac9aa`** until the operator says otherwise.
4. **The #56/#57 conflict is real integration work, not a trivial rebase.**
   - `lib.rs` has 2 conflicting hunks (the plugin handler and `install_global_hotkey`), because
     #56's `dispatch_hotkey_signal` and #57's `apply_hotkey_signal` were two dispatch paths.
   - `hotkeys/mod.rs` has 3 additive hunks, and `hotkeys/AGENTS.md` had an invariant-number collision.
   - All three PRs edit the `PROPOSED_OK` line in `scripts/check-adr-status.sh`.
   - All of these are resolved on the integration branch (item 6).
5. **Windows T6 hotkey.** Right-Alt is broken on Windows in `main` and in #57; only #56 fixes it.
   Windows runs T6 with **Ctrl+Space** against `main` and #57, and with Ctrl+Space **and Right-Alt**
   on the integration branch.
6. **Integration branch: `test/integrate-56-57`** on `Navigata1/kaydence`. It is #57 `9cac9aa` with
   #56 merged in, using one shared hotkey dispatch path (details are in the PR #57 comment).
   - Dictate on it (T6, T7) before anything merges.
   - The fork's CI does **not** run on plain branch pushes, so the Windows build there is the first
     Windows compile of this merge.
7. **No 7-Zip on Windows.** T5 there reads Tauri's generated installer scripts, and runs
   `kaydence-ctl record status` from the build output.
8. **Evidence-branch rule (replaces §5 step 1).**
   - Only the Linux session pushes to the #57 and #58 branches.
   - Mac and Windows push their evidence files to their own `evidence/<platform>-…` branches, and
     comment on the PRs.
   - Linux folds those files into #57 and #58 during the post-merge rebases.
9. **Idle RAM.** Mac and Windows report idle RAM for `main` vs #58. Linux went up ~40 MB, because the
   decoding state now stays resident. The Mac's Metal lane was already over budget on `main`.

## Why this exists

Two draft PRs are CI-green on macOS, Windows and Linux, and are proven live on Omarchy:
- **PR #57** (ADR-0023): the Linux lane.
- **PR #58** (ADR-0024): one Whisper pass per dictation, with a fitted encoder window.

The operator must decide both ADRs before merging. CI cannot prove two things the decisions depend on:

1. **ADR-0024 accuracy on your hardware.** CI never runs the golden transcription test, and Metal and
   ARM64/x64 SIMD can behave differently from Linux x86.
   - **Field warning:** voxtype (the Omarchy dictation tool) shipped the same trick in January.
   - Users on **large-v3-turbo + GPU** got repeated-phrase loops on 3–8 s phrases, so voxtype switched
     it off by default (voxtype #124, #130).
   - PR #58 already includes the fixes voxtype landed on (`no_context`, a floor of 640 frames, a +128 margin), but only you can check GPU lanes.
2. **PR #57 on Mac/Windows.** It refactors the shared hotkey-edge code (`lib.rs`) and changes which
   binaries go into every installer. CI builds it but never dictates with it.

## Context pack: read these first

Everything in the **GitHub** column can be read by any agent. The **live page** column has the same
content in the operator's claude.ai pages, which open only when signed in as the operator. The public
copies leave out operator-private notes.

| What | GitHub (any agent) | Live page (operator) |
|---|---|---|
| This handoff | this file · [HTML version](2026-09-27-mac-windows-adr-verification.html) | [claude.ai/artifact/G5oiJw4UwEmJ2un6ZhEcHt](https://claude.ai/artifact/G5oiJw4UwEmJ2un6ZhEcHt) |
| **Linux state of the union + plan of attack** | [context/…-linux-sotu-poa.md](context/2026-09-27-kaydence-linux-sotu-poa.md) · [.html](context/2026-09-27-kaydence-linux-sotu-poa.html) | [claude.ai/artifact/LK9HNQanhuZ5VuMLiaAzma](https://claude.ai/artifact/LK9HNQanhuZ5VuMLiaAzma) |
| **Build-vs-borrow study** (voxtype, OSTT, Handy, OpenWhispr, Linux tools) | [context/…-build-vs-borrow.md](context/2026-09-27-kaydence-build-vs-borrow.md) · [.html](context/2026-09-27-kaydence-build-vs-borrow.html) | [claude.ai/artifact/Ww4VsL1t6SMaYG9owKFDRL](https://claude.ai/artifact/Ww4VsL1t6SMaYG9owKFDRL) |
| PR #57 (Linux lane, ADR-0023) | [IslandDevCrew/kaydence#57](https://github.com/IslandDevCrew/kaydence/pull/57) · [ADR-0023](https://github.com/Navigata1/kaydence/blob/mission/p1-linux-omarchy-lane/docs/decisions/0023-omarchy-hyprland-linux-lane.md) · [evidence](https://github.com/Navigata1/kaydence/blob/mission/p1-linux-omarchy-lane/ops/mission/evidence/2026-09-25-omarchy-linux-lane.txt) (§10.12 = 26 Sep evening) | — |
| PR #58 (ASR speed, ADR-0024) | [IslandDevCrew/kaydence#58](https://github.com/IslandDevCrew/kaydence/pull/58) · [ADR-0024](https://github.com/Navigata1/kaydence/blob/mission/p1-g3-whisper-audio-ctx/docs/decisions/0024-fit-whisper-encoder-window.md) · [evidence](https://github.com/Navigata1/kaydence/blob/mission/p1-g3-whisper-audio-ctx/ops/mission/evidence/2026-09-26-p1-g3-whisper-audio-ctx.txt) | — |

**Your platform's own comparison data is already on `main`**
([evidence folder](https://github.com/IslandDevCrew/kaydence/tree/main/ops/mission/evidence)):
- **macOS:**
  - `2026-07-10-p1-g2-golden-asr.txt`
  - `2026-07-10-p1-g3-whisper-metal-warmup.txt`
  - `2026-07-11-p1-g3-macos-reference-bench.txt`, plus the `-local-cpu` and `-local-gpu` JSON files
  - `2026-07-11-p1-g3-macos-packaged-background-footprint.txt`
- **Windows:**
  - `2026-07-09-windows-injection.txt`
  - `2026-07-11-p1-g3-windows-arm64-build-contract.txt`
  - `2026-07-11-p1-g3-windows-q5-reference-bench.txt`, plus the five run JSON files
- **Project-wide:** `ops/mission/state-of-the-union.html`, `docs/ROADMAP.md`, and any plans or SOTUs kept locally on your machine.

**Ask:** add a short **"Linux vs this platform"** table to your evidence file. Cover:
- release → text p95, `main` vs `pr58`;
- idle RAM;
- typing method;
- password refusal;
- hotkey path;
- anything your older plans flag as open.

The operator can then compare all three platforms side by side.

## 0. Rules (from AGENTS.md; non-negotiable)

- No push or merge to `main`. No ADR `Status:` changes. No edits to `ops/mission/state.json` or `journal.md`.
- Write only **new** files under `ops/mission/evidence/` on the PR branches, plus PR comments.
- Live typing only into a throwaway editor window you opened (TextEdit / Notepad).
- Password fields only for the refusal test.
- Use a single measurement owner per machine: no parallel benches and no broad `pkill`.
- Never print secrets. The only model is the pinned one below.

## 1. What to run

| PR / ADR | Question | Tests | Decision-grade pass |
|---|---|---|---|
| **#58 / ADR-0024** | Are transcripts unchanged or better on Metal / ARM64 / x64, with no loops, and faster? | **T1** engine A/B · **T2** reference bench A/B · **T3** live smoke · T4 large-v3-turbo (recommended) | No word regressions against `main` on any lane; no repetition loops; faster; bench p95 ≤ 1,200 ms |
| **#57 / ADR-0023** | Does Mac/Windows still dictate, and do the installers ship the right binaries? | **T5** bundle contents · **T6** live hotkey dictation · **T7** password-field refusal | All pass, with no change from `main` |

## 2. Setup (both machines)

```bash
cd <your kaydence clone>
git fetch origin main
git fetch origin pull/57/head:pr-57 pull/58/head:pr-58
git worktree add ../kd-main origin/main      # baseline, expect f5f4eb6
git worktree add ../kd-pr57 pr-57            # expect 9cac9aa
git worktree add ../kd-pr58 pr-58            # expect acddbdd (record the SHA if it moved)
```

**Tip:** `main` and `pr-58` share one `Cargo.lock`. Set `CARGO_TARGET_DIR` to a single shared directory
and run them one after the other, so whisper.cpp compiles only once.

**Model:**
- File: `ggml-base.en-q5_1.bin`, 59,721,011 B, sha256
  `4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f`.
- Both machines already have it from the July runs. If missing:
  `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q5_1.bin`.
- Verify it with `shasum -a 256` or `Get-FileHash`.

**Clips** (byte-identical on every OS):
1. Download `https://github.com/ggml-org/whisper.cpp/raw/master/samples/jfk.wav`. Expect sha256
   `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e`, 352,078 B.
2. Save this as `cut-wav.mjs` and run the four commands below.

```js
// node cut-wav.mjs in.wav out.wav startSample endSample  (16 kHz mono PCM16, canonical header)
import { readFileSync, writeFileSync } from "node:fs";
const [, , src, dst, s, e] = process.argv; const buf = readFileSync(src); let off = 12, data;
while (off + 8 <= buf.length) { const id = buf.toString("ascii", off, off + 4), n = buf.readUInt32LE(off + 4);
  if (id === "data") { data = buf.subarray(off + 8, off + 8 + n); break; } off += 8 + n + (n & 1); }
const pcm = data.subarray(Number(s) * 2, Number(e) * 2), h = Buffer.alloc(44);
h.write("RIFF", 0); h.writeUInt32LE(36 + pcm.length, 4); h.write("WAVEfmt ", 8); h.writeUInt32LE(16, 16);
h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22); h.writeUInt32LE(16000, 24); h.writeUInt32LE(32000, 28);
h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34); h.write("data", 36); h.writeUInt32LE(pcm.length, 40);
writeFileSync(dst, Buffer.concat([h, pcm]));
```

| Clip | Command args | sha256 (first 16) | Reference text |
|---|---|---|---|
| c1 11.0 s | `jfk.wav c1.wav 0 176000` | `d7d4e74b8a333ed0` | And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country. |
| c2 4.9 s (bench clip) | `jfk.wav c2.wav 48000 126400` | `5b0bcef3052a0b45` | Ask not what your country can do for you. |
| c3 1.48 s (the regression case) | `jfk.wav c3.wav 48000 71680` | `98313ce1ead6d45d` | Ask not. |
| c4 2.88 s | `jfk.wav c4.wav 0 46080` | `2a5acd8f08c7e5b2` | And so my fellow Americans. |

**Mac only, then copy to Windows:** also regenerate the July synthetic corpus. These very short clips
are the riskiest case for ADR-0024.

```bash
for t in "hello there" "send the report" "the quick brown fox jumps over the lazy dog" "open the pull request and merge it"; do
  f="s-$(echo "$t" | tr ' ' '-' | cut -c1-24)"; say -v Samantha -o "$f.aiff" "$t"
  ffmpeg -v error -y -i "$f.aiff" -ar 16000 -ac 1 -c:a pcm_s16le "$f.wav"; done
```

## 3. PR #58 / ADR-0024

### T1: Engine A/B (required)
Run each clip × {`kd-main`, `kd-pr58`} × each lane. From `apps/desktop/src-tauri`:

```bash
# Mac, Metal lane:  --features asr-whisper-metal   (no LANE var; the output must say lane=LocalGpu)
# Mac, CPU lane:    --features asr-whisper  with KAYDENCE_WHISPER_LANE=cpu
KAYDENCE_WHISPER_MODEL=$MODEL KAYDENCE_WHISPER_CLIP=$CLIP KAYDENCE_WHISPER_LANE=cpu GGML_NATIVE=OFF \
  cargo test --release --features asr-whisper --test asr_golden -- --nocapture 2>&1 | grep -E "final_text|metrics"
```
```powershell
# Windows (CPU lane only; ARM64: use the July VsDevCmd/clang-cl environment from
# ops/mission/evidence/2026-07-11-p1-g3-windows-q5-reference-bench.txt on main)
$env:GGML_NATIVE="OFF"; $env:KAYDENCE_WHISPER_MODEL=$Model; $env:KAYDENCE_WHISPER_LANE="cpu"
foreach ($c in Get-ChildItem $Clips\*.wav) { $env:KAYDENCE_WHISPER_CLIP=$c.FullName
  cargo test --release --features asr-whisper --test asr_golden -- --nocapture 2>&1 |
    Select-String "final_text|metrics" | ForEach-Object { "$($c.Name)  $_" } }
```

- On `main` the test **fails its 1,200 ms assertion on slow lanes. That is expected**; record the
  text and time anyway.
- On c1 the model says "asked not" on **both** sides. That is a known base.en edge case, not a regression.

**Linux x86 reference row** (i7-8650U, CPU, same model), `main` → `pr58`:

| Clip | main | pr58 | Words |
|---|---|---|---|
| c1 | 1,941 ms | **732 ms** | same |
| c2 | 1,837 ms | **627 ms** | same |
| c3 | 1,820 ms "Ask not!" | **670 ms** "ask not" | same (punctuation only) |
| c4 | 1,805 ms | **631 ms** | same |

**Pass criteria:**
- Normalize both outputs (lowercase, strip punctuation). pr58 must have the same words as main, or be
  closer to the reference text.
- **No repetition**: fail if any 3+ word phrase repeats back-to-back, or the output has more than 1.5×
  the reference word count.
- No empty output.
- pr58 is at least as fast as main on every lane.

### T2: Reference bench A/B (required, full pipeline including VAD + join)
```bash
cargo build --release --features asr-whisper[-metal] --bin reference-bench     # in each worktree
KAYDENCE_REFERENCE_BIN=<target>/release/reference-bench KAYDENCE_WHISPER_MODEL=$MODEL KAYDENCE_WHISPER_CLIP=c2.wav \
KAYDENCE_WHISPER_MODEL_ID=whisper-base-en-q5_1 \
KAYDENCE_WHISPER_SHA256=4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f \
KAYDENCE_WHISPER_LANE=cpu node scripts/bench-reference.mjs | tail -1        # repo root; also LANE=gpu on Mac
```
- Record `release_to_delivery_policy_p50_ms` / `_p95_ms`, `idle_ram_mb`, `idle_cpu_pct` and `status`.
- Linux x86: p95 4,062 → **742** ms; RAM 107.6 → ~147 MB. The RAM rise is expected: the decoding state is now kept resident.
- The Mac Metal lane already failed the RAM budget on `main` in July, so record the delta; don't gate on it.
- **Pass:** pr58 p95 ≤ 1,200 and ≤ main.

### T3: Live smoke (required)
Run the app from `kd-pr58` the way you normally do on this machine, with the Whisper feature and
base.en q5_1 selected. Dictate 3 phrases into TextEdit/Notepad:
1. A short "Yes."
2. One sentence with a **deliberate 1-second pause** in the middle.
3. About 15 s of normal speech.

**Pass:** the text is right, arrives as one sentence (no fragment punctuation such as "Ask not! What…"),
and lands in about 1 s.

### T4: large-v3-turbo loop check (recommended, Mac Metal)
This is the combination behind voxtype's loop reports.
- Get `ggml-large-v3-turbo-q5_0.bin` from the same Hugging Face repo, and **record its sha256**; the registry still says `TODO`.
- Repeat T1 on the Metal lane with this model for c2, c3, c4 and the synthetic clips.
- **Any repetition = FAIL**; report it before anyone accepts ADR-0024.

## 4. PR #57 / ADR-0023

### T5: Bundle contents (required)
```bash
pnpm install && pnpm --filter kaydence-desktop build
cd apps/desktop && cargo tauri build --features asr-whisper-metal   # Windows: --features asr-whisper
```
Inspect the output:
- **macOS:** `Kaydence.app/Contents/MacOS/`.
- **Windows:** the installer. Use `msiexec /a <msi> /qn TARGETDIR=C:\tmp\kd` or 7-Zip on the NSIS exe.
- Do the same on `kd-main` for comparison.

**Expected:** only `kaydence` and `kaydence-ctl`, with no `*-selftest` and no `reference-bench`. `main`
ships one stray selftest; that is the bug #57 fixes.
- Run `kaydence-ctl record status`. Expect exactly `the record control command is only available on
  Linux`, a non-zero "unsupported" exit, and no crash. The helper is inert on Mac/Windows; note it as a
  follow-up to stop shipping it there.

### T6: Live hotkey dictation (required)
Run the `kd-pr57` app. Use the configured push-to-talk hotkey:
- **Hold-to-talk twice:** "testing kaydence on mac/windows", into TextEdit/Notepad.
- **One quick tap** under 250 ms: it must be discarded (pitfall P1).
- **Toggle mode once**, if configured.

**Pass:** the text lands every time and behaves the same as on `main`.

### T7: Password-field refusal (required)
- Repeat the July refusal check: a secure/password field must get **nothing** and the app must show a hold.
- Evidence to match: Windows `2026-07-09-windows-injection.txt` (UIA `IsPassword`); macOS
  `AXSecureTextField`.

## 5. Hand back

1. Write the evidence files and push them to the PR branch. These are fork branches; maintainer pushes
   are allowed.
   - #58: `ops/mission/evidence/2026-09-27-adr0024-<macos-arm64|windows-arm64|windows-x64>.txt`
   - #57: `ops/mission/evidence/2026-09-27-adr0023-<platform>-smoke.txt`
   - Include: host and toolchain, the three SHAs, exact commands, the T-tables, and a verdict per test.
   - Commit as `test(evidence): …`.
   - If the push is refused: `gh pr comment <57|58> --repo IslandDevCrew/kaydence --body-file <file>`.
2. Post a **handoff manifest** as a PR comment, using the format in `docs/protocol/AGENT_INTEROP_PROTOCOL.md` §3:
   - `claim_status` is `verified`, `falsified` or `blocked`;
   - never report "blocked" as "passed".

**Result table to paste** (one row per clip × lane × model):

| Machine | Lane | Model | Clip | main ms | pr58 ms | main text | pr58 text | Words same? | Loop? |
|---|---|---|---|---|---|---|---|---|---|

## 6. How the operator decides

- **ADR-0023 → "go on 0023"** if T5–T7 pass on the Mac and on Windows. Then:
  1. Commit the acceptance to the branch (as with ADR-0021).
  2. Mark the PR ready.
  3. Merge with a merge commit, in the order **#56 → #57 → #58**, rebasing between merges.
- **ADR-0024 → "go on 0024"** if T1–T3 pass on every lane and T4 shows no loops.
- **If a lane regresses or loops, do not merge.** The options are:
  - **Recommended:** add voxtype's hardening, which is to round the window up to a multiple of 8 and
    rerun at the full window whenever the output looks degenerate;
  - raise the floor for that lane;
  - limit the fitted window to CPU lanes.

  Then re-run that lane only.
