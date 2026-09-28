# Kaydence Windows Codex Handoff

Date: 2026-07-11T08:20Z

This handoff supersedes `docs/WINDOWS_CODEX_HANDOFF_2026-07-09.md`.

## Current Truth

- Repo: `https://github.com/IslandDevCrew/kaydence.git` (private).
- Latest confirmed remote main: `80c93748f43032a93398b75efb24d71a38a5af45`.
- Draft PR #11 carries truthful Whisper warmup, portable release defaults, and
  Linux runtime proof. Draft PR #12 is stacked on #11 and closes Linux
  human-focus injection proof.
- Draft PR #13 (`codex/p1-g3-windows-asr-runtime`) is stacked on #12 and
  carries this Windows ARM64 runtime proof.
- GitHub Actions is not running new jobs. PR #11 runs `29142969657` and
  `29143051942` failed before checkout with zero setup steps because the account
  reports failed payments or an exhausted spending limit. This is not a code
  test result, and none of the draft PRs may merge without a fresh full matrix.

## Windows Runtime Proof

The production `local_asr_stack() -> WhisperCppEngine` path passed on a real
Windows 11 Pro ARM64 VM:

- Source: exact archive of commit `a5dc909`.
- Model sha256:
  `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002`.
- Clip sha256:
  `2b1aa6d79c35e790927cafdb7f5b10b81543895e5f7e4b7951761612348d3895`.
- Exact transcript: `Hello there`.
- Warmup: 513 ms.
- Warm LocalCpu inference: 1071 ms, under the 1200 ms hard gate.
- Requested GPU without compiled acceleration truthfully returned `LocalCpu`.
- Test: 1 passed, 0 failed; batch exit code 0.
- Evidence:
  `ops/mission/evidence/2026-07-11-p1-g3-windows-whisper-runtime.txt`.

## Windows ARM64 Build Contract

Current whisper.cpp rejects MSVC for ARM. Windows ARM64 therefore needs the
official Visual Studio Build Tools LLVM components:

- `Microsoft.VisualStudio.Component.VC.Llvm.Clang`
- `Microsoft.VisualStudio.Component.VC.Llvm.ClangToolset`

Verified environment:

```cmd
call C:\BuildTools\Common7\Tools\VsDevCmd.bat -arch=arm64 -host_arch=arm64
set PATH=C:\BuildTools\VC\Tools\Llvm\ARM64\bin;C:\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;%PATH%
set CMAKE_GENERATOR=Ninja
set CMAKE_C_COMPILER=C:\BuildTools\VC\Tools\Llvm\ARM64\bin\clang-cl.exe
set CMAKE_CXX_COMPILER=C:\BuildTools\VC\Tools\Llvm\ARM64\bin\clang-cl.exe
set CMAKE_ASM_COMPILER=C:\BuildTools\VC\Tools\Llvm\ARM64\bin\clang-cl.exe
set CXXFLAGS=/EHsc
set LIBCLANG_PATH=C:\BuildTools\VC\Tools\Llvm\ARM64\bin
set GGML_NATIVE=OFF
```

Do not set `WHISPER_DONT_GENERATE_BINDINGS` on Windows ARM64. The bundled
whisper-rs-sys bindings contain incompatible glibc layout assertions; native
libclang must generate target-correct bindings.

This contract is now encoded in `scripts/windows-arm64-build.ps1` and guarded
by `tests/windows-arm64-build-contract.ps1` on every Windows CI leg. Windows
PowerShell 5.1 may run as an x64 process on ARM64 Windows, so the script uses
the Rust host plus parsed PE machine fields as architecture authority. It
requires AA64 clang/libclang/link, permits the official x86 Ninja host tool,
rejects any bindings opt-out, and runs locked Cargo with an explicit ARM64 target.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  scripts\windows-arm64-build.ps1 -BuildToolsPath C:\BuildTools -Profile release
```

Real VM proof: the review-fixed explicit-target production build completed in
2m38s at `target\aarch64-pc-windows-msvc\release\kaydence.exe`. The script's
PE check and independent `dumpbin` both report `AA64 machine (ARM64)` for the
12,018,176-byte executable, whose sha256 is
`ba0e358927bc4e1eebff2c2e25d6d3cf64059c7c5f9c0da5af60e73090da89c5`.
Evidence: `ops/mission/evidence/2026-07-11-p1-g3-windows-arm64-build-contract.txt`.

## Windows x64 Build Prerequisite

The shipped `asr-whisper` feature runs bindgen, which needs `libclang.dll`.
Install official LLVM (`winget install --id LLVM.LLVM -e`; bindgen finds
`C:\Program Files\LLVM\bin` without `LIBCLANG_PATH`) or the VS Build Tools
`Microsoft.VisualStudio.Component.VC.Llvm.Clang` component. An LLVM-MinGW
`clang.exe` on PATH is not enough: it ships `libclang-cpp.dll` only, and the
build fails with "Unable to find libclang". Hosted `windows-latest` runners
already include LLVM. Verified on an x64 NUC 2026-09-24.

## Still Needed On Windows

1. Integrate the proven ARM64 release-build primitive into P3 `.msi` packaging,
   signing, checksums, and release publication before claiming packaging readiness.
2. Complete P1-G4 with one passing Windows WebView proof that captures Boards
   01/03/07/10 plus the permissions modal, then obtain final human signoff.
3. Prove the real first-run journey: model install, microphone/privacy state,
   UIA/SendInput permission evidence, hotkey recovery, first dictation, and
   setup completion in 60 seconds or less.
4. Measure full hotkey-release-to-inject p50/p95 plus ASR-resident RAM and idle
   CPU. The current proof measures only warm ASR inference.
5. Add the independent Parakeet/ort runtime and human-voice WER corpus.
6. Done: the close-to-tray race fix merged 2026-09-24 (PR #55, ADR-0021
   Accepted). macOS/Linux should re-run their close/reopen/Quit proofs on the new
   path. Evidence: `ops/mission/evidence/2026-09-24-p1-g3-windows-close-race.txt`.
7. Merge the Windows Right-Alt hotkey (ADR-0022, Proposed): `RightAlt` never
   registered via `global-hotkey` ("Unknown VKCode for AltRight"), and mapping it
   would register but never fire. A Raw Input listener with a `vkE8` menu mask and
   chord stop now serves it; live-proven on the x64 NUC. Evidence:
   `ops/mission/evidence/2026-09-24-p1-p0-1-windows-right-alt.txt`.

## Do Not Claim

- P1-G3 is not complete: release-to-inject, resident RAM/CPU, reference p95,
  and the no-model 86 MB process-group gap remain open.
- A real Windows ARM64 runtime pass is not a fresh hosted 3-OS matrix.
- Opening Windows Settings is not permission readiness.
- The model downloader remains a separate operator-gated network decision.
