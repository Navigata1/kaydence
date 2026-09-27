# KAYDENCE — Root Agent Charter

> **Read this file before touching anything in this repository.** Every directory
> with meaningful logic has its own `AGENTS.md` that inherits from this one.
> Directory-level files may *tighten* these rules; they may never loosen them.
>
> **Product name:** Kaydence (primary domain `kaydence.io`). *Cadence* was the
> working title — retired over a software trademark conflict (Cadence Design
> Systems). The brand string is never hardcoded; use `APP_NAME` from `settings/`.

---

## 1. What this project is

**Kaydence** is a local-first, cross-platform (macOS + Windows + Linux) AI
dictation app. One hotkey, speak, clean text appears in any application.

**North Star:** *Speak anywhere. Ship clean text. Never leak, never lose, never lag.*

It exists because the market split into two camps that each solve half the problem:

| Camp | Examples | Wins | Loses |
|---|---|---|---|
| Cloud AI dictation | Wispr Flow, Aqua Voice, Willow | Cleanup quality, tone, speed | Privacy trust, RAM/CPU bloat, subscriptions, offline death |
| Local tools | Handy, VoiceInk, Superwhisper, OS built-ins | Privacy, cost, offline | Raw output ("edit tax"), latency, Mac-only or no polish |

Kaydence is the deliberate middle: **local-first engine, optional intelligence
layer, Windows and Linux as first-class citizens** — plus a predictive completion
layer (**Whisper-Ahead**) and a flagship trio (**Voiceprint**, **Relay**,
**Conductor**; PRD P4, ADR-0009) nobody else ships cross-platform. Pricing is
pay-once (Core/Pro/Captain) + optional self-hostable Harbor services (ADR-0010).
Evidence base:
`docs/COMPETITIVE-ANALYSIS.md`. Requirements: `docs/PRD.md`. Build protocol:
`prompts/BUILD-LOOP.md`.

## 2. Non-negotiables (the constitution)

Violating any of these is a blocking defect regardless of who or what requested it.

1. **Privacy is structural, and the line is transmission + persistence — not
   awareness.** No telemetry. No accounts. No data ever leaves the device, and
   nothing is persisted without consent. The app MAY read context locally to
   improve predictions (surrounding text via accessibility, or optional
   on-device OCR) **only when the user explicitly enables it** — and such context
   is held **in memory only, never written to disk, never transmitted**. No
   cloud screen capture of any kind, ever (this is the Wispr-Flow failure mode;
   permanently banned, no toggle). Password/secure fields are always blocked.
   Network calls happen only for features the user turned on (BYOK ASR/cleanup,
   model downloads, update checks) and only to the endpoint they configured. See
   ADR-0006.
2. **Never lose a word.** Audio is persisted to local disk *before* transcription
   begins (write-ahead). A crash, failed paste, or focus change must never
   destroy what the user said. History keeps raw audio + raw transcript + cleaned
   transcript until the user deletes them.
3. **Cleanup is a dial, not a default.** Raw / Light / Full. Light (filler
   removal + self-correction collapsing + punctuation, no rewriting) is default.
   Full rewrite is opt-in. Users hate the edit tax of raw output AND hate AI that
   erases their voice.
4. **Latency budgets are requirements.** See §5. A feature that blows the budget
   does not merge.
5. **Windows and Linux are first-class.** Nothing ships, merges, or is "done" if
   it works on macOS only, or on macOS + Windows only. CI runs all three
   platforms (macOS + Windows + Linux); platform-specific code lives behind
   traits. Linux injection covers **both X11 and Wayland** (ADR-0011). *Host
   caveat (2026-07-07): the current build host is macOS-only with no remote, so
   the Windows/Linux CI legs are waived-pending-infra — the mandate stands, only
   its verification is deferred; see `ops/mission/state.json` blockers.*
6. **Local always works.** Every cloud-assisted path has a local fallback that
   degrades gracefully (cloud ASR → local GPU → local CPU; cloud cleanup → local
   Ollama → rule-engine Light → Raw; cloud prediction never — prediction is local
   only).
7. **Lightweight or nothing.** Tauri, not Electron. Idle RAM < 250 MB with the
   ASR model resident, < 80 MB without. The prediction model, when enabled,
   carries its own additional budget (§5). Idle CPU < 1%. Installer < 60 MB
   excluding models (one exception: the Linux AppImage, which bundles its own
   WebKitGTK/GTK to run on any distro, may reach 100 MB — operator decision
   2026-09-26, ADR-0023; native .deb/.rpm/AUR packages stay < 60 MB).
8. **Never inject into secure fields.** Detect password/secure inputs and refuse,
   surfacing a notification instead.

## 3. Repository map

```
kaydence/
├── AGENTS.md              ← you are here (constitution)
├── README.md
├── SCAFFOLD.md            ← Phase 0 toolchain bootstrap
├── prompts/              ← the autonomous build system
│   ├── KICKOFF.md         · the single-command entry point
│   ├── BUILD-LOOP.md      · orient→plan→build→judge→audit→gate→loop
│   └── JUDGE-AUDITOR.md   · the two evidence lenses + critical decision paths
├── .claude/commands/     ← Claude Code slash commands (/kickoff, /recap, /audit)
├── docs/                  ← PRD, architecture, roadmap, ADRs, research
├── apps/desktop/          ← the Tauri application
│   ├── src-tauri/         ← Rust backend (the real product)
│   │   └── src/{audio,engine,cleanup,prediction,context,inject,hotkeys,
│   │             dictionary,profiles,history,settings}
│   └── src/               ← React frontend (settings, dual-line HUD, history)
├── crates/                ← shared Rust crates (extract on second consumer)
├── models/                ← registry.json + download/verify (no weights in git)
├── scripts/               ← bench, bench-prediction, audit-network, release
├── tests/integration/     ← cross-module pipeline tests
└── .github/workflows/     ← CI: lint + test + latency gate on macOS, Windows AND Linux
```

## 4. The pipeline (memorize this)

```
Hotkey → Audio capture (cpal) → WAL persist → Silero VAD
                 │
                 ▼
   ASR engine (streaming partials)            ┌── context/ (opt-in, in-memory)
   ├─ Parakeet V3 (CPU)                       │   surrounding text → prediction
   ├─ Whisper large-v3-turbo (GPU)            ▼
   └─ BYOK cloud (opt-in)          prediction/ → small local LLM → surface router
                 │                              ├─ Surface A: HUD dual-line (blue lane)
                 ▼                              └─ Surface B: inline ghost (paused-only)
   Cleanup (Raw / Light / Full)                       │
                 │                       merge (say / Tab) ──► commits (gold pulse)
                 ▼                                     └────► prediction analytics (local)
   Dictionary (terms, snippets)
                 │
                 ▼
   Injection (native; clipboard fallback) ◄──── only COMMITTED text injects
                 │
                 ▼
   History (audio + raw + cleaned, local SQLite)
```

Module boundaries follow this exactly. Stages communicate only via the typed
`SessionEvent` contract (`docs/ARCHITECTURE.md` §4); no stage imports another's
internals.

## 5. Latency & footprint budgets

| Measurement | Budget |
|---|---|
| Hotkey press → capture starts | ≤ 50 ms |
| Streaming partial text lag behind speech | ≤ 300 ms |
| Key release → Raw final injected (local GPU) | ≤ 700 ms |
| Key release → Raw final injected (local CPU / Parakeet) | ≤ 1,200 ms |
| Additional cost of Light cleanup | ≤ 800 ms |
| Prediction: pause → guess rendered (HUD, streaming) | ≤ 400 ms |
| Idle RAM (ASR resident) / idle CPU | ≤ 250 MB / ≤ 1% |
| Added RAM when prediction model enabled | ≤ 1.5 GB (model-dependent; documented per model) |

`scripts/bench.sh` and `scripts/bench-prediction.sh` measure these; CI fails the
PR if a budget regresses >10%. **Dictation always wins:** if prediction threatens
a dictation budget, prediction yields.

## 6. Pitfall registry (competitors' scars — do not repeat them)

| # | Pitfall | Scar | Our rule |
|---|---|---|---|
| P1 | Hotkey races truncate short utterances / return empty | VoiceInk issues #686/#687/#696 | Min 250 ms capture; 300 ms tail buffer; debounce; <1 s test |
| P2 | Clipboard-paste injection destroys the clipboard | paste-based tools | Native first; fallback snapshots & restores ≤200 ms |
| P3 | Idle resource hogging (~800 MB / 8% CPU) | Wispr Electron | Budgets in §5, enforced in CI |
| P4 | Over-polished output — "the soul is gone" | Wispr rewrite backlash | Cleanup dial; Light default; Full never silent |
| P5 | Cloud-only death / wifi-dependent latency | Aqua, Willow | Non-negotiable #6 fallback chain |
| P6 | Privacy overreach: cloud screenshots, login items, telemetry | Wispr trust collapse (2.7) | Non-negotiable #1; `scripts/audit-network.sh` |
| P7 | Lost recordings on crash/interrupt | praised when absent (Aqua) | Non-negotiable #2 WAL |
| P8 | Windows second-class (freezes, late ports) | Wispr/Glaido/Superwhisper | Non-negotiable #5; CI matrix |
| P9 | Focus change mid-dictation sends text to wrong app | generic | Bind target at capture start; else Hold + HUD |
| P10 | Setup complexity scares users off | Superwhisper | First run: one screen, dictating in ≤60 s |
| P11 | Prediction crowding the active line is distracting | Cotypist same-line ghost | Prediction on the line below (Surface A) / paused-only inline (Surface B) |
| P12 | Inline streaming in coding apps breaks focus | — | Surface B disables streaming; paused-only after 10 s |

## 7. Agent workflow protocol

This repo runs an **autonomous, evidence-gated build loop** (`prompts/BUILD-LOOP.md`),
launched by a single command (`/kickoff`). Core rules every agent obeys:

1. **Orient.** Read this file, the `AGENTS.md` of every directory you touch, and
   referenced ADRs — every time, regardless of familiarity.
2. **No authority without evidence.** Every change traces to a PRD item
   (`docs/PRD.md` IDs) or approved issue. Every "done" is backed by an artifact
   (test output, bench table, audit result) — never an assertion.
3. **Plan before code** for anything spanning >1 module or touching a
   non-negotiable; record architectural shifts as ADRs.
4. **Command vocabulary** (human operator): `proceed`/`go` (execute agreed plan),
   `recap` (summarize state + next), `clarify` (surface open questions),
   `rewind` (return to last checkpoint, discard since).
5. **Critical decision paths** (touching a non-negotiable, a new dependency, a
   new network surface, platform code, the model registry, or a cleanup/
   prediction prompt contract) require an ADR **and** a human gate before merge.
6. **Model tiering.** Architecture, prompt engineering, `engine/`, `prediction/`,
   `context/`, and `inject/` are frontier-model work. Mechanical refactors, docs,
   and test boilerplate may run smaller — charter review still applies.

### 7.1 Archipelago + multi-agent interop (all agents: Buzz, Codex, Claude)

Trigger phrases — "**arch build**", "**run this through archipelago**", "**my
arch skill**" — mean: run the work through the archipelago methodology
(`github.com/Navigata1/archipelago`): lock the goal as **falsifiable** claims,
lock a plan of phases+gates, then walk an evidence-gated build loop where
**nothing crosses a gate on claims alone**. A falsified claim caps the audit at
band 1; an unverified claim at band 3; a UI claim with no runtime evidence at
band 4. Mark unverified work `unverified` — never launder it into `verified`.

Three planes govern collaboration (full spec:
[`docs/protocol/AGENT_INTEROP_PROTOCOL.md`](docs/protocol/AGENT_INTEROP_PROTOCOL.md)):
**git = durable truth**, **chat/Buzz = dispatch (no authority)**, **CI = the
merge authority**. An agent's chat report is a *pointer*, never a *proof*: the
proof is the committed evidence file + the gate result at a named SHA. Each
assignment is a **work packet**; each completion returns a **handoff manifest**.

**Hard boundaries — no agent (or bridge) does these automatically:** push/merge
to `main`; edit `ops/mission/state.json` or shared evidence outside its packet;
close a human gate; change an ADR status; flip repo visibility. Physical-device
measurement runs single-owner (one `OWNER_LOCK`; no concurrent `pkill`).

The merge is automated the archipelago way — see
[`docs/protocol/GATE_MERGE_FUSION.md`](docs/protocol/GATE_MERGE_FUSION.md): CI is
made a required check, PRs opt into auto-merge, and a PR lands *because a check
that could have failed didn't* — so "an agent never merges on claims" still holds
and the human is never the merge bottleneck.

## 8. Definition of Done (every PR)

- [ ] Builds and tests pass on **macOS, Windows, and Linux** (Win/Linux via CI once infra lands; see mission blockers)
- [ ] New logic has unit tests; pipeline changes have an integration test
- [ ] Latency budgets verified if the change touches the hot path
- [ ] No new network calls (or: ADR + user-toggle + audit allowlist updated)
- [ ] Relevant `AGENTS.md` updated if conventions/invariants changed
- [ ] PRD item referenced in the PR description
- [ ] Judge pass (quality/charter) **and** Audit pass (evidence) recorded — see `prompts/JUDGE-AUDITOR.md`
- [ ] Reader test: a fresh agent given only your diff + the relevant `AGENTS.md` can explain what changed and why

## 9. Style & conventions (global)

- **Rust:** stable, `cargo fmt` + `clippy -D warnings`. Errors via `thiserror`
  per module; no `unwrap()` outside tests. Audio thread is real-time, never
  blocks on async.
- **TypeScript/React:** strict, no `any`. Frontend is presentation only — all
  logic in Rust. Event types generated from Rust, never hand-written.
- **Commits:** conventional commits (`feat(prediction): …`, `fix(inject): …`).
- **Branding:** never hardcode "Kaydence" — use `APP_NAME` from `settings/`.
