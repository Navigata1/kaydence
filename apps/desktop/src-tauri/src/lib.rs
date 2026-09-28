//! Kaydence backend library — the real product (root `AGENTS.md`).
//!
//! Wires the pipeline modules and the Tauri app shell. The runtime path is:
//! hotkeys -> audio(+VAD) -> engine -> cleanup ->
//! dictionary -> profiles -> inject -> history, with prediction/context as a
//! parallel local-only layer. Stages couple only through `events::SessionEvent`.

#[cfg(desktop)]
use std::path::{Path, PathBuf};
#[cfg(desktop)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(desktop)]
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(desktop)]
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(desktop)]
use tauri::Manager;

pub mod events;

pub mod audio;
pub mod bench;
pub mod cleanup;
pub mod context;
pub mod dictionary;
pub mod engine;
pub mod history;
pub mod hotkeys;
pub mod inject;
pub mod models;
pub mod pipeline;
pub mod prediction;
pub mod profiles;
pub mod settings;

use crate::events::CleanupDial;

pub fn run_bench_json() -> Result<String, bench::BenchError> {
    bench::run_bench_json()
}

/// Initial app/config snapshot for the presentation layer.
#[tauri::command]
fn app_snapshot(state: tauri::State<'_, RuntimeSnapshot>) -> settings::AppSnapshot {
    state.snapshot()
}

#[tauri::command]
fn select_asr_model(
    app: tauri::AppHandle,
    model_id: String,
    state: tauri::State<'_, RuntimeSnapshot>,
    runtime: tauri::State<'_, HotkeyRuntimeHandle>,
) -> Result<settings::AppSnapshot, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        runtime
            .ensure_idle_for_asr_update()
            .map_err(|err| err.to_string())?;
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        state.set_settings_store(&app_data_dir);
        state
            .select_asr_model(&model_id)
            .map_err(|err| err.to_string())?;
        let snapshot = state
            .refresh_models(&models::source_tree_registry_path(), &app_data_dir)
            .map_err(|err| err.to_string())?;
        complete_asr_runtime_update(
            &state,
            apply_selected_asr_to_runtime(&snapshot, &app_data_dir, &runtime),
        )
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = runtime;
        state
            .select_asr_model(&model_id)
            .map_err(|err| err.to_string())
    }
}

#[tauri::command]
fn set_hotkey_mode(
    app: tauri::AppHandle,
    state: tauri::State<'_, RuntimeSnapshot>,
    runtime: tauri::State<'_, HotkeyRuntimeHandle>,
    mode: String,
) -> Result<settings::AppSnapshot, String> {
    let mode = settings::HotkeyModeSetting::parse(&mode).map_err(|err| err.to_string())?;

    #[cfg(desktop)]
    {
        use tauri::Manager;

        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        state
            .set_hotkey_mode(mode, &app_data_dir, &runtime)
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = runtime;
        Ok(state.apply_hotkey_mode(mode))
    }
}

#[tauri::command]
fn set_hotkey_binding(
    app: tauri::AppHandle,
    state: tauri::State<'_, RuntimeSnapshot>,
    runtime: tauri::State<'_, HotkeyRuntimeHandle>,
    binding: String,
) -> Result<settings::AppSnapshot, String> {
    let binding = settings::normalize_hotkey_binding(&binding).map_err(|err| err.to_string())?;

    #[cfg(desktop)]
    {
        use tauri::Manager;

        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        state
            .set_hotkey_binding(&binding, &app, &app_data_dir, &runtime)
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = runtime;
        Ok(state.apply_hotkey_binding(&binding))
    }
}

#[tauri::command]
fn set_cleanup_dial(
    app: tauri::AppHandle,
    state: tauri::State<'_, RuntimeSnapshot>,
    runtime: tauri::State<'_, HotkeyRuntimeHandle>,
    dial: String,
) -> Result<settings::AppSnapshot, String> {
    let dial = settings::parse_cleanup_dial(&dial).map_err(|err| err.to_string())?;

    #[cfg(desktop)]
    {
        use tauri::Manager;

        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        state
            .set_cleanup_dial(dial, &app_data_dir, &runtime)
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = runtime;
        Ok(state.apply_cleanup_dial(dial))
    }
}

#[tauri::command]
fn refresh_model_readiness(
    app: tauri::AppHandle,
    state: tauri::State<'_, RuntimeSnapshot>,
    runtime: tauri::State<'_, HotkeyRuntimeHandle>,
) -> Result<settings::AppSnapshot, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        runtime
            .ensure_idle_for_asr_update()
            .map_err(|err| err.to_string())?;
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        let snapshot = state
            .refresh_models_from_app_data(&app_data_dir)
            .map_err(|err| err.to_string())?;
        complete_asr_runtime_update(
            &state,
            apply_selected_asr_to_runtime(&snapshot, &app_data_dir, &runtime),
        )
    }

    #[cfg(not(desktop))]
    {
        Ok(state.snapshot())
    }
}

#[tauri::command]
fn install_model_artifact(
    app: tauri::AppHandle,
    state: tauri::State<'_, RuntimeSnapshot>,
    runtime: tauri::State<'_, HotkeyRuntimeHandle>,
    model_id: String,
    source_path: String,
) -> Result<settings::AppSnapshot, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        runtime
            .ensure_idle_for_asr_update()
            .map_err(|err| err.to_string())?;
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        let snapshot = state
            .install_model_artifact(
                &models::source_tree_registry_path(),
                &app_data_dir,
                &model_id,
                &PathBuf::from(source_path),
            )
            .map_err(|err| err.to_string())?;
        complete_asr_runtime_update(
            &state,
            apply_selected_asr_to_runtime(&snapshot, &app_data_dir, &runtime),
        )
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = runtime;
        let _ = model_id;
        let _ = source_path;
        Ok(state.snapshot())
    }
}

#[tauri::command]
fn first_run_model_download_preflight(
    app: tauri::AppHandle,
    state: tauri::State<'_, RuntimeSnapshot>,
    model_id: String,
) -> Result<settings::FirstRunModelDownloadPreflight, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        state
            .model_download_preflight(
                &models::source_tree_registry_path(),
                &app_data_dir,
                &model_id,
            )
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = state;
        let _ = model_id;
        Err("Model download preflight requires the desktop runtime".to_string())
    }
}

/// Result of an actual first-run model fetch (P1-P0-8, ADR-0017).
#[derive(Debug, Clone, serde::Serialize)]
pub struct FirstRunModelDownloadResult {
    pub model_id: String,
    pub path: String,
    pub size_bytes: u64,
    pub snapshot: settings::AppSnapshot,
}

/// Perform the first-run "model download" step: fetch a registry-pinned model
/// through the reviewed downloader (ADR-0017) and verify sha256 before install.
/// Feature-gated `model-download` — the default build has no fetch surface and
/// returns an explicit unavailable error rather than silently doing nothing.
#[tauri::command]
async fn first_run_model_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, RuntimeSnapshot>,
    runtime: tauri::State<'_, HotkeyRuntimeHandle>,
    model_id: String,
) -> Result<FirstRunModelDownloadResult, String> {
    #[cfg(all(desktop, feature = "model-download"))]
    {
        use tauri::Manager;

        runtime
            .ensure_idle_for_asr_update()
            .map_err(|err| err.to_string())?;
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        let registry = models::ModelRegistry::load(&models::source_tree_registry_path())
            .map_err(|err| err.to_string())?;
        let entry = registry
            .require(model_id.trim())
            .map_err(|err| err.to_string())?
            .clone();
        let models_dir = app_data_dir.join("models");
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            models::download::download_and_install(&entry, &models_dir)
        })
        .await
        .map_err(|err| format!("Model download task failed: {err}"))?
        .map_err(|err| err.to_string())?;
        let snapshot = state
            .refresh_models_from_app_data(&app_data_dir)
            .map_err(|err| err.to_string())?;
        let snapshot = complete_asr_runtime_update(
            &state,
            apply_selected_asr_to_runtime(&snapshot, &app_data_dir, &runtime),
        )?;
        Ok(FirstRunModelDownloadResult {
            model_id: outcome.id,
            path: outcome.path.display().to_string(),
            size_bytes: outcome.size_bytes,
            snapshot,
        })
    }

    #[cfg(not(all(desktop, feature = "model-download")))]
    {
        let _ = (app, state, runtime, model_id);
        Err("Model download is unavailable in this build; enable the `model-download` feature (ADR-0017).".to_string())
    }
}

#[tauri::command]
fn first_run_permission_action(
    requirement_id: String,
) -> Result<settings::FirstRunPermissionActionOutcome, String> {
    settings::first_run_permission_action(&requirement_id).map_err(|err| err.to_string())
}

#[tauri::command]
fn open_first_run_permission_settings(
    requirement_id: String,
) -> Result<settings::FirstRunPermissionSettingsOpenOutcome, String> {
    let action =
        settings::first_run_permission_action(&requirement_id).map_err(|err| err.to_string())?;

    let Some(target) = action.settings_target.as_deref() else {
        return settings::first_run_permission_settings_open_outcome(&requirement_id, false)
            .map_err(|err| err.to_string());
    };

    #[cfg(desktop)]
    {
        open_os_settings_target(target)?;
        settings::first_run_permission_settings_open_outcome(&requirement_id, true)
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = target;
        settings::first_run_permission_settings_open_outcome(&requirement_id, false)
            .map_err(|err| err.to_string())
    }
}

#[cfg(desktop)]
fn open_os_settings_target(target: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("/usr/bin/open");
        command.arg(target);
        run_settings_open_command(&mut command, target)
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", target]);
        run_settings_open_command(&mut command, target)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(target);
        run_settings_open_command(&mut command, target)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
    {
        let _ = target;
        Err(
            "This platform does not expose a settings opener for first-run permissions."
                .to_string(),
        )
    }
}

#[cfg(desktop)]
fn run_settings_open_command(
    command: &mut std::process::Command,
    target: &str,
) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|err| format!("Could not open OS settings target {target}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "OS settings target {target} exited with status {status}"
        ))
    }
}

#[tauri::command]
fn refresh_first_run_runtime_proofs(
    state: tauri::State<'_, RuntimeSnapshot>,
    runtime: tauri::State<'_, HotkeyRuntimeHandle>,
) -> settings::AppSnapshot {
    apply_hotkey_first_run_proof_to_snapshot(&state, runtime.take_first_run_proof());
    apply_platform_permission_proofs_to_snapshot(&state, &inject::platform_permission_proofs());
    state.snapshot()
}

#[tauri::command]
fn export_first_run_proof_plan(
    app: tauri::AppHandle,
    state: tauri::State<'_, RuntimeSnapshot>,
) -> Result<settings::FirstRunProofExportOutcome, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        export_first_run_proof_plan_to_app_data(&state.snapshot(), &app_data_dir, current_unix_ms())
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = state;
        Ok(settings::FirstRunProofExportOutcome {
            exported: false,
            json_path: None,
            item_count: 0,
        })
    }
}

#[tauri::command]
fn recent_history(
    app: tauri::AppHandle,
    state: tauri::State<'_, RuntimeSnapshot>,
    limit: Option<usize>,
) -> Result<Vec<history::HistorySession>, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        let retention_days = state.snapshot().settings.privacy.history_retention_days;
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        let mut store =
            history::HistoryStore::open(&app_data_dir).map_err(|err| err.to_string())?;
        store
            .sweep_retention(retention_days, &app_data_dir)
            .map_err(|err| err.to_string())?;
        recover_history_audio(&mut store, &app_data_dir).map_err(|err| err.to_string())?;
        store
            .list_recent(limit.unwrap_or(5).clamp(1, 20))
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = state;
        let _ = limit;
        Ok(Vec::new())
    }
}

#[cfg(desktop)]
fn recover_history_audio(
    store: &mut history::HistoryStore,
    app_data_dir: &Path,
) -> Result<usize, HistoryRecoveryError> {
    let sessions_dir = app_data_dir.join("sessions");
    if !sessions_dir.exists() {
        return Ok(0);
    }

    let mut surfaced = 0;
    for entry in std::fs::read_dir(&sessions_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("wav") {
            continue;
        };
        let Some(session_id) = session_id_from_wal_path(&path) else {
            continue;
        };
        if store.session_has_audio(session_id)? {
            continue;
        }
        let Ok(recovered) = audio::wal::recover(&path) else {
            continue;
        };
        if store.record_recovered_audio(session_id, &recovered.path, recovered.samples)? {
            surfaced += 1;
        }
    }
    Ok(surfaced)
}

#[cfg(desktop)]
fn session_id_from_wal_path(path: &Path) -> Option<events::SessionId> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| ulid::Ulid::from_string(stem).ok())
        .map(events::SessionId::new)
}

#[cfg(desktop)]
#[derive(Debug, thiserror::Error)]
enum HistoryRecoveryError {
    #[error("history recovery io: {0}")]
    Io(#[from] std::io::Error),
    #[error("audio recovery: {0}")]
    Wal(#[from] audio::wal::WalError),
    #[error("history: {0}")]
    History(#[from] history::HistoryError),
}

#[tauri::command]
fn delete_history_session(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Vec<history::HistorySession>, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        let mut store =
            history::HistoryStore::open(&app_data_dir).map_err(|err| err.to_string())?;
        store
            .delete_session_and_audio(&session_id, &app_data_dir)
            .map_err(|err| err.to_string())?;
        store.list_recent(4).map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = session_id;
        Ok(Vec::new())
    }
}

#[tauri::command]
fn export_history_session(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<history::ExportSessionOutcome, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        let session_id = parse_history_session_id(&session_id)?;
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        let mut store =
            history::HistoryStore::open(&app_data_dir).map_err(|err| err.to_string())?;
        recover_history_audio(&mut store, &app_data_dir).map_err(|err| err.to_string())?;
        store
            .export_session(session_id, &app_data_dir)
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = session_id;
        Ok(history::ExportSessionOutcome {
            exported: false,
            json_path: None,
            text_path: None,
        })
    }
}

#[tauri::command]
fn play_history_audio(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Option<history::HistoryAudioPlayback>, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        let session_id = parse_history_session_id(&session_id)?;
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        let mut store =
            history::HistoryStore::open(&app_data_dir).map_err(|err| err.to_string())?;
        recover_history_audio(&mut store, &app_data_dir).map_err(|err| err.to_string())?;
        store
            .audio_playback(session_id, &app_data_dir)
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = session_id;
        Ok(None)
    }
}

#[tauri::command]
fn purge_history(app: tauri::AppHandle) -> Result<history::PurgeHistoryOutcome, String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("App data directory unavailable: {err}"))?;
        let mut store =
            history::HistoryStore::open(&app_data_dir).map_err(|err| err.to_string())?;
        store
            .purge_all(&app_data_dir)
            .map_err(|err| err.to_string())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        Ok(history::PurgeHistoryOutcome {
            sessions_deleted: 0,
            audio_files_removed: 0,
            export_files_removed: 0,
        })
    }
}

fn parse_history_session_id(session_id: &str) -> Result<events::SessionId, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("History session id cannot be empty".to_string());
    }
    ulid::Ulid::from_string(session_id)
        .map(events::SessionId::new)
        .map_err(|_| format!("Invalid history session id: {session_id}"))
}

#[cfg(desktop)]
fn export_first_run_proof_plan_to_app_data(
    snapshot: &settings::AppSnapshot,
    app_data_dir: &Path,
    generated_at_ms: u64,
) -> Result<settings::FirstRunProofExportOutcome, FirstRunProofExportError> {
    let plan = settings::first_run_proof_plan(snapshot, generated_at_ms);
    let exports_dir = app_data_dir.join(history::EXPORTS_DIR);
    std::fs::create_dir_all(&exports_dir)?;
    let json_path = exports_dir.join("first-run-proof-plan.json");
    std::fs::write(&json_path, serde_json::to_string_pretty(&plan)?)?;

    Ok(settings::FirstRunProofExportOutcome {
        exported: true,
        json_path: Some(json_path.display().to_string()),
        item_count: plan.proof_items.len(),
    })
}

#[cfg(desktop)]
#[derive(Debug, thiserror::Error)]
enum FirstRunProofExportError {
    #[error("first-run proof export io: {0}")]
    Io(#[from] std::io::Error),
    #[error("first-run proof export json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
struct RuntimeSnapshot {
    inner: Mutex<settings::AppSnapshot>,
    settings_store: Mutex<Option<settings::SettingsStore>>,
}

#[derive(Default)]
struct HotkeyRuntimeHandle {
    #[cfg(desktop)]
    inner: Mutex<Option<Arc<Mutex<HotkeyRuntime>>>>,
    #[cfg(desktop)]
    primary_shortcut: Mutex<Option<tauri_plugin_global_shortcut::Shortcut>>,
    #[cfg(desktop)]
    cleanup_override_shortcut: Mutex<Option<tauri_plugin_global_shortcut::Shortcut>>,
    /// Outcome of starting the Windows Right-Alt Raw Input listener
    /// (ADR-0022); `None` until install tried.
    #[cfg(target_os = "windows")]
    native_listener: Mutex<Option<Result<(), String>>>,
}

#[cfg(desktop)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HotkeyShortcutRole {
    Primary,
    CleanupOverride,
}

#[cfg(desktop)]
impl HotkeyRuntimeHandle {
    fn set_runtime(&self, runtime: Arc<Mutex<HotkeyRuntime>>) {
        *self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(runtime);
    }

    fn set_primary_shortcut(&self, shortcut: tauri_plugin_global_shortcut::Shortcut) {
        *self
            .primary_shortcut
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(shortcut);
    }

    fn set_cleanup_override_shortcut(
        &self,
        shortcut: Option<tauri_plugin_global_shortcut::Shortcut>,
    ) {
        *self
            .cleanup_override_shortcut
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = shortcut;
    }

    fn active_shortcut(&self) -> Option<tauri_plugin_global_shortcut::Shortcut> {
        *self
            .primary_shortcut
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn cleanup_override_shortcut(&self) -> Option<tauri_plugin_global_shortcut::Shortcut> {
        *self
            .cleanup_override_shortcut
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn shortcut_role(
        &self,
        observed: &tauri_plugin_global_shortcut::Shortcut,
    ) -> Option<HotkeyShortcutRole> {
        if self
            .active_shortcut()
            .is_some_and(|primary| primary == *observed)
        {
            return Some(HotkeyShortcutRole::Primary);
        }
        if self
            .cleanup_override_shortcut()
            .is_some_and(|override_shortcut| override_shortcut == *observed)
        {
            return Some(HotkeyShortcutRole::CleanupOverride);
        }
        None
    }

    /// The role a Right-Alt press currently maps to, via the same bound
    /// shortcuts the plugin path uses, so rebinding works unchanged. Only the
    /// Windows Raw Input listener needs it (ADR-0022).
    #[cfg(any(target_os = "windows", test))]
    fn right_alt_role(&self, shift: bool) -> Option<HotkeyShortcutRole> {
        use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

        let mods = shift.then_some(Modifiers::SHIFT);
        self.shortcut_role(&Shortcut::new(mods, Code::AltRight))
    }

    /// Register with the OS plugin, or confirm the native listener that owns
    /// this binding instead (ADR-0022).
    fn register_shortcut<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        shortcut: tauri_plugin_global_shortcut::Shortcut,
    ) -> Result<(), String> {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;

        if served_by_native_listener(&shortcut) {
            return self.native_listener_status();
        }
        app.global_shortcut()
            .register(shortcut)
            .map_err(|err| err.to_string())
    }

    fn unregister_shortcut<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        shortcut: tauri_plugin_global_shortcut::Shortcut,
    ) -> Result<(), String> {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;

        if served_by_native_listener(&shortcut) {
            return Ok(());
        }
        app.global_shortcut()
            .unregister(shortcut)
            .map_err(|err| err.to_string())
    }

    #[cfg(target_os = "windows")]
    fn native_listener_status(&self) -> Result<(), String> {
        self.native_listener
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or_else(|| Err("Right-Alt listener has not started".to_string()))
    }

    #[cfg(not(target_os = "windows"))]
    fn native_listener_status(&self) -> Result<(), String> {
        Err("no native hotkey listener on this platform".to_string())
    }

    #[cfg(target_os = "windows")]
    fn start_right_alt_listener(
        &self,
        app: tauri::AppHandle,
        runtime: Arc<Mutex<HotkeyRuntime>>,
        started: Instant,
    ) {
        let sink = Arc::new(RightAltRuntimeSink {
            app,
            runtime,
            started,
            hold_role: Mutex::new(None),
        });
        let status =
            hotkeys::windows::spawn_right_alt_listener(sink).map_err(|err| err.to_string());
        if let Err(err) = &status {
            eprintln!("Kaydence {err}");
        }
        *self
            .native_listener
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(status);
    }

    fn ensure_idle(&self) -> Result<(), HotkeyBindingUpdateError> {
        let Some(runtime) = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        else {
            return Ok(());
        };

        let runtime = runtime
            .lock()
            .map_err(|_| HotkeyBindingUpdateError::RuntimePoisoned)?;
        if runtime.is_idle() {
            Ok(())
        } else {
            Err(HotkeyBindingUpdateError::CaptureActive)
        }
    }

    fn ensure_idle_for_asr_update(&self) -> Result<(), AsrRuntimeUpdateError> {
        let Some(runtime) = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        else {
            return Ok(());
        };

        let runtime = runtime
            .lock()
            .map_err(|_| AsrRuntimeUpdateError::RuntimePoisoned)?;
        if runtime.is_idle() {
            Ok(())
        } else {
            Err(AsrRuntimeUpdateError::CaptureActive)
        }
    }

    fn apply_asr_adapter_state(
        &self,
        state: engine::LocalAsrAdapterState,
    ) -> Result<Option<engine::EngineLane>, AsrRuntimeUpdateError> {
        let Some(runtime) = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        else {
            return Ok(None);
        };

        let mut runtime = runtime
            .lock()
            .map_err(|_| AsrRuntimeUpdateError::RuntimePoisoned)?;
        runtime.set_asr_adapter_state(state)
    }

    fn apply_cleanup_dial(&self, dial: CleanupDial) -> Result<(), CleanupDialUpdateError> {
        let Some(runtime) = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        else {
            return Ok(());
        };

        let mut runtime = runtime
            .lock()
            .map_err(|_| CleanupDialUpdateError::RuntimePoisoned)?;
        runtime.set_default_cleanup_dial(dial)
    }

    fn take_first_run_proof(&self) -> HotkeyRuntimeFirstRunProof {
        let Some(runtime) = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        else {
            return HotkeyRuntimeFirstRunProof::default();
        };

        let proof = match runtime.lock() {
            Ok(mut runtime) => runtime.take_first_run_proof(),
            Err(_) => HotkeyRuntimeFirstRunProof::default(),
        };
        proof
    }

    fn apply_binding<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        binding: &str,
    ) -> Result<String, HotkeyBindingUpdateError> {
        let binding = settings::normalize_hotkey_binding(binding)?;
        let shortcut = shortcut_from_binding(&binding)?;
        if self
            .cleanup_override_shortcut()
            .is_some_and(|override_shortcut| override_shortcut == shortcut)
        {
            return Err(HotkeyBindingUpdateError::Register(
                "primary hotkey cannot match the cleanup override chord".to_string(),
            ));
        }
        if self.active_shortcut() == Some(shortcut) {
            return Ok(binding);
        }

        self.ensure_idle()?;
        self.register_shortcut(app, shortcut)
            .map_err(HotkeyBindingUpdateError::Register)?;

        if let Some(previous) = self.active_shortcut() {
            if let Err(err) = self.unregister_shortcut(app, previous) {
                eprintln!("Kaydence old global hotkey unregister failed after rebind: {err}");
            }
        }
        self.set_primary_shortcut(shortcut);
        Ok(binding)
    }

    fn apply_mode(
        &self,
        mode: settings::HotkeyModeSetting,
        capture: settings::CaptureSettings,
    ) -> Result<(), HotkeyModeUpdateError> {
        let Some(runtime) = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        else {
            return Ok(());
        };

        let mut runtime = runtime
            .lock()
            .map_err(|_| HotkeyModeUpdateError::RuntimePoisoned)?;
        runtime.set_hotkey_mode(mode, capture)
    }
}

impl Default for RuntimeSnapshot {
    fn default() -> Self {
        Self {
            inner: Mutex::new(settings::AppSnapshot::default()),
            settings_store: Mutex::new(None),
        }
    }
}

impl RuntimeSnapshot {
    fn snapshot(&self) -> settings::AppSnapshot {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    #[cfg(desktop)]
    fn mark_hotkey_registered(&self) {
        self.update_first_run(|first_run| {
            first_run.hotkey_registered = true;
            first_run.hotkey_registration_error = None;
        });
    }

    #[cfg(desktop)]
    fn mark_input_permission_ready(&self) {
        self.update_first_run(|first_run| {
            first_run.input_permission_ready = true;
            settings::sync_first_run_permission_requirements(first_run);
        });
    }

    #[cfg(desktop)]
    fn mark_microphone_permission_ready(&self) {
        self.update_first_run(|first_run| {
            first_run.microphone_permission_ready = true;
            settings::sync_first_run_permission_requirements(first_run);
        });
    }

    #[cfg(desktop)]
    fn mark_permission_requirement_ready(
        &self,
        requirement_id: &str,
        detail: &str,
        action: &str,
    ) -> bool {
        let mut marked = false;
        self.update_first_run(|first_run| {
            if let Some(requirement) = first_run
                .permission_requirements
                .iter_mut()
                .find(|requirement| requirement.id == requirement_id)
            {
                requirement.state = settings::FirstRunPermissionState::Ready;
                requirement.detail = detail.to_string();
                requirement.action = action.to_string();
                if requirement_id == "input_monitoring" {
                    first_run.input_permission_ready = true;
                }
                marked = true;
            }
        });
        marked
    }

    #[cfg(desktop)]
    fn mark_hotkey_registration_failed(&self, error: String) {
        self.update_first_run(|first_run| {
            first_run.hotkey_registered = false;
            first_run.hotkey_registration_error = Some(error);
        });
    }

    #[cfg(desktop)]
    fn mark_first_dictation_completed(
        &self,
    ) -> Result<settings::AppSnapshot, settings::SettingsStoreError> {
        self.mark_first_dictation_completed_at(current_unix_ms())
    }

    #[cfg(desktop)]
    fn mark_first_dictation_completed_at(
        &self,
        completed_at_ms: u64,
    ) -> Result<settings::AppSnapshot, settings::SettingsStoreError> {
        if let Some(store) = self.settings_store() {
            let mut settings = store.load()?;
            settings.first_dictation_completed = true;
            if settings.first_dictation_completed_at_ms.is_none() {
                settings.first_dictation_completed_at_ms = Some(completed_at_ms);
            }
            let setup_timing = settings::FirstRunSetupTiming::from_user_settings(&settings);
            store.save(&settings)?;
            self.update_first_run(|first_run| {
                first_run.first_dictation_completed = true;
                first_run.setup_timing = setup_timing;
                first_run.microphone_permission_ready = true;
                first_run.input_permission_ready = true;
                settings::sync_first_run_permission_requirements(first_run);
            });
        } else {
            self.update_first_run(|first_run| {
                first_run.first_dictation_completed = true;
                if first_run.setup_timing.completed_at_ms.is_none() {
                    first_run.setup_timing = settings::FirstRunSetupTiming::from_parts(
                        first_run.setup_timing.started_at_ms,
                        Some(completed_at_ms),
                    );
                }
                first_run.microphone_permission_ready = true;
                first_run.input_permission_ready = true;
                settings::sync_first_run_permission_requirements(first_run);
            });
        }

        Ok(self.snapshot())
    }

    #[cfg(desktop)]
    fn set_settings_store(&self, app_data_dir: &Path) {
        *self
            .settings_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(settings::SettingsStore::new(app_data_dir));
    }

    #[cfg(desktop)]
    fn apply_persisted_user_settings(&self) -> Result<(), settings::SettingsStoreError> {
        let Some(store) = self.settings_store() else {
            return Ok(());
        };
        let settings = store.load()?;
        let setup_timing = settings::FirstRunSetupTiming::from_user_settings(&settings);
        let first_dictation_completed = settings.first_dictation_completed;
        if let Some(model_id) = settings.selected_asr_model_id {
            let _ = self.apply_asr_selection(&model_id);
        }
        if let Some(mode) = settings.hotkey_mode {
            self.apply_hotkey_mode(mode);
        }
        if let Some(binding) = settings.hotkey_primary_binding {
            let binding = settings::normalize_hotkey_binding(&binding)?;
            self.apply_hotkey_binding(&binding);
        }
        if let Some(dial) = settings.cleanup_default_dial {
            self.apply_cleanup_dial(dial);
        }
        self.update_first_run(|first_run| {
            first_run.setup_timing = setup_timing;
            if first_dictation_completed {
                first_run.first_dictation_completed = true;
                first_run.microphone_permission_ready = true;
                first_run.input_permission_ready = true;
                settings::sync_first_run_permission_requirements(first_run);
            }
        });
        Ok(())
    }

    fn apply_hotkey_mode(&self, mode: settings::HotkeyModeSetting) -> settings::AppSnapshot {
        {
            let mut snapshot = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            snapshot.settings.hotkey.mode = mode;
        }
        self.snapshot()
    }

    fn apply_hotkey_binding(&self, binding: &str) -> settings::AppSnapshot {
        {
            let mut snapshot = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            snapshot.settings.hotkey.primary_binding = binding.to_string();
        }
        self.snapshot()
    }

    fn apply_cleanup_dial(&self, dial: CleanupDial) -> settings::AppSnapshot {
        {
            let mut snapshot = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            snapshot.settings.cleanup.default_dial = dial;
        }
        self.snapshot()
    }

    #[cfg(desktop)]
    fn set_hotkey_mode(
        &self,
        mode: settings::HotkeyModeSetting,
        app_data_dir: &Path,
        runtime: &HotkeyRuntimeHandle,
    ) -> Result<settings::AppSnapshot, SetHotkeyModeError> {
        self.set_settings_store(app_data_dir);
        let capture = self.snapshot().settings.capture;
        runtime.apply_mode(mode, capture)?;

        if let Some(store) = self.settings_store() {
            Self::persist_hotkey_mode(&store, mode)?;
        }

        Ok(self.apply_hotkey_mode(mode))
    }

    #[cfg(desktop)]
    fn persist_hotkey_mode(
        store: &settings::SettingsStore,
        mode: settings::HotkeyModeSetting,
    ) -> Result<(), settings::SettingsStoreError> {
        let mut settings = store.load()?;
        settings.hotkey_mode = Some(mode);
        store.save(&settings)
    }

    #[cfg(desktop)]
    fn set_cleanup_dial(
        &self,
        dial: CleanupDial,
        app_data_dir: &Path,
        runtime: &HotkeyRuntimeHandle,
    ) -> Result<settings::AppSnapshot, SetCleanupDialError> {
        self.set_settings_store(app_data_dir);
        runtime.apply_cleanup_dial(dial)?;

        if let Some(store) = self.settings_store() {
            Self::persist_cleanup_dial(&store, dial)?;
        }

        Ok(self.apply_cleanup_dial(dial))
    }

    #[cfg(desktop)]
    fn persist_cleanup_dial(
        store: &settings::SettingsStore,
        dial: CleanupDial,
    ) -> Result<(), settings::SettingsStoreError> {
        let mut settings = store.load()?;
        settings.cleanup_default_dial = Some(dial);
        store.save(&settings)
    }

    #[cfg(desktop)]
    fn persist_hotkey_binding(
        store: &settings::SettingsStore,
        binding: &str,
    ) -> Result<(), settings::SettingsStoreError> {
        let mut settings = store.load()?;
        settings.hotkey_primary_binding = Some(settings::normalize_hotkey_binding(binding)?);
        store.save(&settings)
    }

    #[cfg(desktop)]
    fn ensure_first_run_started_at(
        &self,
        started_at_ms: u64,
    ) -> Result<(), settings::SettingsStoreError> {
        let Some(store) = self.settings_store() else {
            return Ok(());
        };
        let mut settings = store.load()?;
        if settings.first_run_started_at_ms.is_none() && !settings.first_dictation_completed {
            settings.first_run_started_at_ms = Some(started_at_ms);
            store.save(&settings)?;
        }
        let setup_timing = settings::FirstRunSetupTiming::from_user_settings(&settings);
        self.update_first_run(|first_run| {
            first_run.setup_timing = setup_timing;
        });
        Ok(())
    }

    #[cfg(desktop)]
    fn set_hotkey_binding<R: tauri::Runtime>(
        &self,
        binding: &str,
        app: &tauri::AppHandle<R>,
        app_data_dir: &Path,
        runtime: &HotkeyRuntimeHandle,
    ) -> Result<settings::AppSnapshot, SetHotkeyBindingError> {
        self.set_settings_store(app_data_dir);
        let binding = match runtime.apply_binding(app, binding) {
            Ok(binding) => binding,
            Err(err) => {
                if runtime.active_shortcut().is_none() {
                    self.mark_hotkey_registration_failed(err.to_string());
                }
                return Err(err.into());
            }
        };

        if let Some(store) = self.settings_store() {
            Self::persist_hotkey_binding(&store, &binding)?;
        }

        self.mark_hotkey_registered();
        Ok(self.apply_hotkey_binding(&binding))
    }

    #[cfg(all(desktop, test))]
    fn refresh_model_readiness(&self, registry_path: &Path, models_dir: &Path) {
        let selected_asr_model_id = self.snapshot().settings.first_run.selected_asr_model_id;
        self.refresh_model_readiness_for_selection(
            registry_path,
            models_dir,
            selected_asr_model_id.as_deref(),
        );
    }

    #[cfg(desktop)]
    fn refresh_model_readiness_for_selection(
        &self,
        registry_path: &Path,
        models_dir: &Path,
        selected_asr_model_id: Option<&str>,
    ) {
        let readiness = first_run_model_readiness(registry_path, models_dir, selected_asr_model_id);
        self.update_first_run(|first_run| {
            first_run.model_ready = readiness.model_ready;
            first_run.model_readiness_error = readiness.model_readiness_error;
            first_run.required_models = readiness.required_models;
            first_run.asr_candidates = readiness.asr_candidates;
            first_run.recommended_asr_model_id = readiness.recommended_asr_model_id;
            first_run.selected_asr_model_id = readiness.selected_asr_model_id;
            let asr_state = selected_asr_runtime_state(first_run, registry_path, models_dir);
            first_run.asr_runtime = first_run_asr_runtime_status(&asr_state);
        });
    }

    #[cfg(desktop)]
    fn refresh_asr_runtime_status(
        &self,
        registry_path: &Path,
        models_dir: &Path,
    ) -> settings::AppSnapshot {
        self.update_first_run(|first_run| {
            let asr_state = selected_asr_runtime_state(first_run, registry_path, models_dir);
            first_run.asr_runtime = first_run_asr_runtime_status(&asr_state);
        });
        self.snapshot()
    }

    #[cfg(desktop)]
    fn refresh_models(
        &self,
        registry_path: &Path,
        app_data_dir: &Path,
    ) -> Result<settings::AppSnapshot, settings::SettingsStoreError> {
        self.set_settings_store(app_data_dir);
        self.ensure_first_run_started_at(current_unix_ms())?;
        let selected_asr_model_id = self
            .settings_store()
            .map(|store| store.load())
            .transpose()?
            .and_then(|settings| settings.selected_asr_model_id);
        self.refresh_model_readiness_for_selection(
            registry_path,
            &app_data_dir.join("models"),
            selected_asr_model_id.as_deref(),
        );
        self.apply_persisted_user_settings()?;
        Ok(self.refresh_asr_runtime_status(registry_path, &app_data_dir.join("models")))
    }

    #[cfg(desktop)]
    fn refresh_models_from_app_data(
        &self,
        app_data_dir: &Path,
    ) -> Result<settings::AppSnapshot, settings::SettingsStoreError> {
        self.refresh_models(&models::source_tree_registry_path(), app_data_dir)
    }

    #[cfg(desktop)]
    fn install_model_artifact(
        &self,
        registry_path: &Path,
        app_data_dir: &Path,
        model_id: &str,
        source_path: &Path,
    ) -> Result<settings::AppSnapshot, InstallModelArtifactError> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(InstallModelArtifactError::EmptyModelId);
        }

        self.set_settings_store(app_data_dir);
        let registry = models::ModelRegistry::load(registry_path)?;
        let model = registry.require(model_id)?;
        model.install_artifact_from_path(source_path, &app_data_dir.join("models"))?;
        Ok(self.refresh_models(registry_path, app_data_dir)?)
    }

    #[cfg(desktop)]
    fn model_download_preflight(
        &self,
        registry_path: &Path,
        app_data_dir: &Path,
        model_id: &str,
    ) -> Result<settings::FirstRunModelDownloadPreflight, ModelDownloadPreflightError> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(ModelDownloadPreflightError::EmptyModelId);
        }

        let registry = models::ModelRegistry::load(registry_path)?;
        let model = registry.require(model_id)?;
        let snapshot_status = self.first_run_model_status_for(model_id);
        let (state, detail) = snapshot_status.unwrap_or_else(|| {
            (
                settings::FirstRunModelState::Blocked,
                "Model readiness has not been refreshed for this artifact.".to_string(),
            )
        });
        let plan = model.download_plan(&app_data_dir.join("models"));

        let (
            available,
            destination_path,
            expected_sha256,
            size_mb,
            source_count,
            sources,
            blocked_reason,
            operator_action,
            proof_requirement,
        ) = match plan {
            Ok(plan) => {
                let source_count = plan.sources.len().min(usize::from(u16::MAX)) as u16;
                (
                    true,
                    Some(plan.destination_path.display().to_string()),
                    Some(plan.sha256),
                    Some(plan.size_mb),
                    source_count,
                    plan.sources,
                    None,
                    if state == settings::FirstRunModelState::Ready {
                        "No download needed; the local artifact already verifies.".to_string()
                    } else {
                        "Fetch only through the reviewed downloader, then verify sha256 before marking ready."
                            .to_string()
                    },
                    "Download proof must record source URL, destination path, expected sha256, and post-fetch verification."
                        .to_string(),
                )
            }
            Err(err) => (
                false,
                None,
                None,
                None,
                0,
                Vec::new(),
                Some(err.to_string()),
                "Review models/registry.json and replace placeholder hashes or sources with audited artifact metadata."
                    .to_string(),
                "Do not fetch bytes until the registry can produce a validated HTTPS download plan."
                    .to_string(),
            ),
        };

        Ok(settings::FirstRunModelDownloadPreflight {
            model_id: model.id.clone(),
            task: model_task_label(model.task).to_string(),
            lane: effective_model_lane_label(model),
            runtime: model.runtime.clone(),
            file: model.file.clone(),
            state,
            detail,
            available,
            destination_path,
            expected_sha256,
            size_mb,
            source_count,
            sources,
            license: model.license.clone(),
            license_review_required: model.license_review_required,
            blocked_reason,
            operator_action,
            proof_requirement,
        })
    }

    #[cfg(desktop)]
    fn first_run_model_status_for(
        &self,
        model_id: &str,
    ) -> Option<(settings::FirstRunModelState, String)> {
        let snapshot = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let first_run = &snapshot.settings.first_run;
        first_run
            .required_models
            .iter()
            .find(|model| model.id == model_id)
            .map(|model| (model.state, model.detail.clone()))
            .or_else(|| {
                first_run
                    .asr_candidates
                    .iter()
                    .find(|model| model.id == model_id)
                    .map(|model| (model.state, model.detail.clone()))
            })
    }

    #[cfg(desktop)]
    fn mark_model_readiness_failed(&self, error: String) {
        self.update_first_run(|first_run| {
            first_run.model_ready = false;
            first_run.model_readiness_error = Some(error);
            first_run.required_models.clear();
            first_run.asr_candidates.clear();
            first_run.recommended_asr_model_id = None;
            first_run.selected_asr_model_id = None;
            first_run.asr_runtime = settings::FirstRunAsrRuntimeStatus::default();
        });
    }

    fn select_asr_model(&self, model_id: &str) -> Result<settings::AppSnapshot, SelectModelError> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(SelectModelError::EmptyModelId);
        }
        if !self.has_asr_candidate(model_id) {
            return Err(SelectModelError::UnknownModel(model_id.to_string()));
        }

        if let Some(store) = self.settings_store() {
            let mut settings = store.load()?;
            settings.selected_asr_model_id = Some(model_id.to_string());
            store.save(&settings)?;
        }

        self.apply_asr_selection(model_id)
            .ok_or_else(|| SelectModelError::UnknownModel(model_id.to_string()))?;
        Ok(self.snapshot())
    }

    fn has_asr_candidate(&self, model_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .settings
            .first_run
            .asr_candidates
            .iter()
            .any(|candidate| candidate.id == model_id)
    }

    fn apply_asr_selection(&self, model_id: &str) -> Option<()> {
        let mut snapshot = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let first_run = &mut snapshot.settings.first_run;
        if !first_run
            .asr_candidates
            .iter()
            .any(|candidate| candidate.id == model_id)
        {
            return None;
        }

        first_run.selected_asr_model_id = Some(model_id.to_string());
        let mut selected_lane = None;
        for candidate in &mut first_run.asr_candidates {
            candidate.selected = candidate.id == model_id;
            if candidate.selected {
                selected_lane = candidate.lane.clone();
            }
        }
        first_run.asr_runtime =
            first_run_pending_asr_runtime_status(Some(model_id.to_string()), selected_lane);
        first_run.recompute_next_step();
        Some(())
    }

    fn settings_store(&self) -> Option<settings::SettingsStore> {
        self.settings_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    #[cfg(desktop)]
    fn update_first_run(&self, update: impl FnOnce(&mut settings::FirstRunStatus)) {
        let mut snapshot = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut snapshot.settings.first_run);
        snapshot.settings.first_run.recompute_next_step();
    }

    #[cfg(desktop)]
    fn mark_asr_adapter_ready(&self, lane: engine::EngineLane) {
        self.update_first_run(|first_run| {
            let runtime = first_run
                .asr_runtime
                .runtime
                .clone()
                .unwrap_or_else(|| "local ASR".to_string());
            first_run.asr_runtime.lane = Some(engine_lane_snapshot_label(lane).to_string());
            first_run.asr_runtime.adapter_ready = true;
            first_run.asr_runtime.detail = format!(
                "Verified {runtime} adapter loaded and warmed on the {} lane.",
                engine_lane_snapshot_label(lane)
            );
            first_run.asr_runtime.proof_requirement =
                "Keep the real golden-clip transcript and lane-specific latency gate green before release."
                    .to_string();
        });
    }

    #[cfg(desktop)]
    fn mark_asr_adapter_warmup_failed(&self, error: &str) {
        self.update_first_run(|first_run| {
            first_run.asr_runtime.state = settings::FirstRunAsrRuntimeState::Blocked;
            first_run.asr_runtime.adapter_ready = false;
            first_run.asr_runtime.detail = format!("Local ASR adapter warmup failed: {error}");
            first_run.asr_runtime.proof_requirement =
                "Resolve the runtime initialization failure and rerun warmup before first dictation."
                    .to_string();
        });
    }
}

#[cfg(desktop)]
fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[derive(Debug, thiserror::Error)]
enum SelectModelError {
    #[error("ASR model id cannot be empty")]
    EmptyModelId,
    #[error("unknown ASR model id: {0}")]
    UnknownModel(String),
    #[error("settings store: {0}")]
    SettingsStore(#[from] settings::SettingsStoreError),
}

#[derive(Debug, thiserror::Error)]
enum HotkeyModeUpdateError {
    #[error("hotkey mode cannot be changed during an active capture")]
    CaptureActive,
    #[error("hotkey runtime lock poisoned")]
    RuntimePoisoned,
}

#[cfg(desktop)]
#[derive(Debug, thiserror::Error)]
enum AsrRuntimeUpdateError {
    #[error("ASR runtime cannot be changed during an active capture")]
    CaptureActive,
    #[error("ASR runtime lock poisoned")]
    RuntimePoisoned,
    #[error("ASR warmup: {0}")]
    Warmup(#[from] pipeline::PipelineError),
}

#[derive(Debug, thiserror::Error)]
enum CleanupDialUpdateError {
    #[error("cleanup dial cannot be changed during an active capture")]
    CaptureActive,
    #[error("hotkey runtime lock poisoned")]
    RuntimePoisoned,
}

#[derive(Debug, thiserror::Error)]
enum SetHotkeyModeError {
    #[error("hotkey runtime: {0}")]
    Runtime(#[from] HotkeyModeUpdateError),
    #[error("settings store: {0}")]
    SettingsStore(#[from] settings::SettingsStoreError),
}

#[derive(Debug, thiserror::Error)]
enum SetCleanupDialError {
    #[error("cleanup dial runtime: {0}")]
    Runtime(#[from] CleanupDialUpdateError),
    #[error("settings store: {0}")]
    SettingsStore(#[from] settings::SettingsStoreError),
}

#[derive(Debug, thiserror::Error)]
enum HotkeyBindingUpdateError {
    #[error("hotkey binding cannot be changed during an active capture")]
    CaptureActive,
    #[error("hotkey runtime lock poisoned")]
    RuntimePoisoned,
    #[error("shortcut registration failed: {0}")]
    Register(String),
    #[error("settings: {0}")]
    Settings(#[from] settings::SettingsError),
}

#[derive(Debug, thiserror::Error)]
enum SetHotkeyBindingError {
    #[error("hotkey binding: {0}")]
    Runtime(#[from] HotkeyBindingUpdateError),
    #[error("settings store: {0}")]
    SettingsStore(#[from] settings::SettingsStoreError),
}

#[derive(Debug, thiserror::Error)]
enum InstallModelArtifactError {
    #[error("model id cannot be empty")]
    EmptyModelId,
    #[error("model registry: {0}")]
    ModelRegistry(#[from] models::ModelRegistryError),
    #[error("settings store: {0}")]
    SettingsStore(#[from] settings::SettingsStoreError),
}

#[derive(Debug, thiserror::Error)]
enum ModelDownloadPreflightError {
    #[error("model id cannot be empty")]
    EmptyModelId,
    #[error("model registry: {0}")]
    ModelRegistry(#[from] models::ModelRegistryError),
}

#[cfg(desktop)]
struct FirstRunModelReadiness {
    model_ready: bool,
    model_readiness_error: Option<String>,
    required_models: Vec<settings::FirstRunModelStatus>,
    asr_candidates: Vec<settings::FirstRunAsrCandidate>,
    recommended_asr_model_id: Option<String>,
    selected_asr_model_id: Option<String>,
}

#[cfg(desktop)]
fn first_run_model_readiness(
    registry_path: &Path,
    models_dir: &Path,
    selected_asr_model_id: Option<&str>,
) -> FirstRunModelReadiness {
    let registry = match models::ModelRegistry::load(registry_path) {
        Ok(registry) => registry,
        Err(err) => {
            return FirstRunModelReadiness {
                model_ready: false,
                model_readiness_error: Some(format!("Model registry unavailable: {err}")),
                required_models: Vec::new(),
                asr_candidates: Vec::new(),
                recommended_asr_model_id: None,
                selected_asr_model_id: None,
            };
        }
    };
    let platform_tag = first_run_platform_tag();
    let recommended_asr = first_run_asr_recommendation(&registry, platform_tag);
    let recommended_asr_model_id = recommended_asr.map(|model| model.id.clone());
    let selected_asr_model_id = selected_asr_model_id
        .and_then(|model_id| {
            registry
                .get(model_id)
                .filter(|model| model.task == models::ModelTask::Asr && model.recommended)
        })
        .map(|model| model.id.clone())
        .or_else(|| recommended_asr_model_id.clone());

    let required_models = registry
        .verify_required_first_run_models(models_dir, selected_asr_model_id.as_deref())
        .into_iter()
        .map(|(model, status)| first_run_model_status(model, status, models_dir))
        .collect::<Vec<_>>();
    let asr_candidates = registry
        .recommended_for(models::ModelTask::Asr)
        .into_iter()
        .map(|model| {
            first_run_asr_candidate(
                model,
                model.verify_artifact(models_dir),
                models_dir,
                selected_asr_model_id.as_deref(),
                recommended_asr_model_id.as_deref(),
                platform_tag,
            )
        })
        .collect::<Vec<_>>();

    let model_ready = !required_models.is_empty()
        && required_models
            .iter()
            .all(|model| model.state == settings::FirstRunModelState::Ready);
    let model_readiness_error = model_readiness_error(&required_models);

    FirstRunModelReadiness {
        model_ready,
        model_readiness_error,
        required_models,
        asr_candidates,
        recommended_asr_model_id,
        selected_asr_model_id,
    }
}

#[cfg(desktop)]
fn first_run_model_status(
    model: &models::ModelEntry,
    status: Result<models::ModelArtifactStatus, models::ModelRegistryError>,
    models_dir: &Path,
) -> settings::FirstRunModelStatus {
    let download_plan = model.download_plan(models_dir);
    let (state, detail) = match status {
        Ok(models::ModelArtifactStatus::Ready { size_bytes, .. }) => (
            settings::FirstRunModelState::Ready,
            format!("Verified artifact, {} MB on disk", bytes_to_mb(size_bytes)),
        ),
        Ok(models::ModelArtifactStatus::Missing { .. }) => match &download_plan {
            Ok(_) => (
                settings::FirstRunModelState::Missing,
                "Download required before first dictation".to_string(),
            ),
            Err(err) => (
                settings::FirstRunModelState::Blocked,
                format!("Download unavailable: {err}"),
            ),
        },
        Err(models::ModelRegistryError::PlaceholderChecksum { .. }) => (
            settings::FirstRunModelState::Blocked,
            "Registry checksum pending".to_string(),
        ),
        Err(models::ModelRegistryError::ChecksumMismatch { quarantined_to, .. }) => (
            settings::FirstRunModelState::Blocked,
            format!(
                "Checksum mismatch; quarantined at {}",
                quarantined_to.display()
            ),
        ),
        Err(err) => (
            settings::FirstRunModelState::Blocked,
            format!("Registry error: {err}"),
        ),
    };

    settings::FirstRunModelStatus {
        id: model.id.clone(),
        task: model_task_label(model.task).to_string(),
        lane: effective_model_lane_label(model),
        runtime: model.runtime.clone(),
        file: model.file.clone(),
        state,
        detail,
        download_available: download_plan.is_ok(),
        download_size_mb: download_plan.as_ref().ok().map(|plan| plan.size_mb),
        download_source_count: download_plan
            .as_ref()
            .ok()
            .map(|plan| plan.sources.len().min(usize::from(u16::MAX)) as u16)
            .unwrap_or(0),
        license: model.license.clone(),
        license_review_required: model.license_review_required,
    }
}

#[cfg(desktop)]
fn first_run_asr_candidate(
    model: &models::ModelEntry,
    status: Result<models::ModelArtifactStatus, models::ModelRegistryError>,
    models_dir: &Path,
    selected_asr_model_id: Option<&str>,
    recommended_asr_model_id: Option<&str>,
    platform_tag: &str,
) -> settings::FirstRunAsrCandidate {
    let base = first_run_model_status(model, status, models_dir);
    let selected = selected_asr_model_id == Some(model.id.as_str());

    settings::FirstRunAsrCandidate {
        id: model.id.clone(),
        lane: effective_model_lane_label(model),
        runtime: model.runtime.clone(),
        size_mb: model.size_mb,
        min_hw: model.min_hw.clone(),
        state: base.state,
        detail: base.detail,
        download_available: base.download_available,
        download_size_mb: base.download_size_mb,
        download_source_count: base.download_source_count,
        selected,
        recommendation: (recommended_asr_model_id == Some(model.id.as_str()))
            .then(|| recommendation_reason(model, platform_tag).to_string()),
        license: model.license.clone(),
        license_review_required: model.license_review_required,
    }
}

#[cfg(desktop)]
fn model_readiness_error(models: &[settings::FirstRunModelStatus]) -> Option<String> {
    if models.is_empty() {
        return Some("No required first-run models are registered".to_string());
    }

    if models
        .iter()
        .any(|model| model.state == settings::FirstRunModelState::Blocked)
    {
        return Some(
            "Model registry needs verified metadata, checksums, or artifact repair".to_string(),
        );
    }

    None
}

#[cfg(desktop)]
fn first_run_asr_recommendation<'a>(
    registry: &'a models::ModelRegistry,
    platform_tag: &str,
) -> Option<&'a models::ModelEntry> {
    let candidates = registry.recommended_for(models::ModelTask::Asr);
    let reviewed_candidates = candidates
        .iter()
        .copied()
        .filter(|model| model.download_plan(Path::new(".")).is_ok())
        .collect::<Vec<_>>();
    let candidates = if reviewed_candidates.is_empty() {
        candidates
    } else {
        reviewed_candidates
    };
    candidates
        .iter()
        .copied()
        .find(|model| model.default_for.iter().any(|tag| tag == platform_tag))
        .or_else(|| {
            candidates.iter().copied().find(|model| {
                model.lane.as_deref() == Some("cpu") || model.min_hw.eq_ignore_ascii_case("any")
            })
        })
        .or_else(|| candidates.first().copied())
}

#[cfg(desktop)]
fn recommendation_reason(model: &models::ModelEntry, platform_tag: &str) -> &'static str {
    if model.default_for.iter().any(|tag| tag == platform_tag) {
        "Recommended for this OS lane"
    } else if model.lane.as_deref() == Some("cpu") || model.min_hw.eq_ignore_ascii_case("any") {
        "CPU-safe first-run default"
    } else {
        "Recommended local ASR option"
    }
}

#[cfg(desktop)]
fn first_run_platform_tag() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "desktop"
    }
}

#[cfg(desktop)]
fn bytes_to_mb(size_bytes: u64) -> u64 {
    size_bytes.div_ceil(1024 * 1024)
}

#[cfg(desktop)]
fn model_task_label(task: models::ModelTask) -> &'static str {
    match task {
        models::ModelTask::Asr => "ASR",
        models::ModelTask::Vad => "VAD",
        models::ModelTask::Cleanup => "Cleanup",
        models::ModelTask::Prediction => "Prediction",
    }
}

#[cfg(desktop)]
struct HotkeyRuntime {
    coordinator: hotkeys::CaptureCoordinator,
    recorder: audio::WalCaptureRuntime,
    history: history::HistoryStore,
    target_resolver: Box<dyn profiles::ResolveSessionTarget + Send>,
    active_target: Option<profiles::SessionTarget>,
    processor: Box<dyn pipeline::CaptureProcessor + Send>,
    injector: Box<dyn inject::TextInjector + Send>,
    unknown_field_policy: inject::UnknownFieldPolicy,
    prefer_clipboard: bool,
    default_cleanup_dial: CleanupDial,
    active_shortcut_role: Option<HotkeyShortcutRole>,
    capture_cleanup_override: Option<CleanupDial>,
    microphone_permission_ready_pending: bool,
    first_dictation_completion_pending: bool,
}

#[cfg(desktop)]
#[derive(Debug, thiserror::Error)]
enum HotkeyRuntimeError {
    #[error("audio: {0}")]
    Audio(#[from] audio::CaptureRuntimeError),
    #[error("history: {0}")]
    History(#[from] history::HistoryError),
    #[error("pipeline: {0}")]
    Pipeline(#[from] pipeline::PipelineError),
}

#[cfg(desktop)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct HotkeyRuntimeFirstRunProof {
    microphone_permission_ready: bool,
    first_dictation_completed: bool,
}

#[cfg(desktop)]
impl HotkeyRuntime {
    fn new(
        app_data_dir: impl Into<PathBuf>,
        settings: &settings::AppSettings,
    ) -> Result<Self, HotkeyRuntimeError> {
        let app_data_dir = app_data_dir.into();
        let recorder = audio::WalCaptureRuntime::new(app_data_dir.clone());
        let history = history::HistoryStore::open(&app_data_dir)?;
        let asr_state = selected_asr_runtime_state(
            &settings.first_run,
            &models::source_tree_registry_path(),
            &app_data_dir.join("models"),
        );
        let mut runtime = Self::with_recorder(
            recorder,
            history,
            Box::new(profiles::platform_target_resolver()),
            Box::new(pipeline::default_runtime_pipeline_with_asr(asr_state)),
            Box::new(inject::platform_injector()),
        );
        runtime.coordinator =
            hotkey_coordinator_from_settings(settings.hotkey.mode, settings.capture);
        runtime.default_cleanup_dial = settings.cleanup.default_dial;
        Ok(runtime)
    }

    #[cfg(test)]
    fn new_wal_only(app_data_dir: impl Into<PathBuf>) -> Result<Self, HotkeyRuntimeError> {
        let app_data_dir = app_data_dir.into();
        let recorder = audio::WalCaptureRuntime::new_wal_only(app_data_dir.clone());
        let history = history::HistoryStore::open(&app_data_dir)?;
        // Inert OS seams: a unit test must never read the live desktop's focus or
        // type into whatever window the developer has focused (the Linux
        // injector is real since ADR-0023). Tests that exercise delivery opt in
        // with `with_target_resolver` / `with_injector` fakes.
        Ok(Self::with_recorder(
            recorder,
            history,
            Box::new(profiles::SessionTargetResolver::new(
                profiles::StaticFrontmostAppDetector::unknown(),
                profiles::ProfileStore::default(),
            )),
            Box::new(pipeline::default_runtime_pipeline_with_asr(
                engine::LocalAsrAdapterState::Pending {
                    selected_model_id: None,
                    lane: engine::EngineLane::LocalCpu,
                },
            )),
            Box::new(inject::UnimplementedInjector { platform: "test" }),
        ))
    }

    fn with_recorder(
        recorder: audio::WalCaptureRuntime,
        history: history::HistoryStore,
        target_resolver: Box<dyn profiles::ResolveSessionTarget + Send>,
        processor: Box<dyn pipeline::CaptureProcessor + Send>,
        injector: Box<dyn inject::TextInjector + Send>,
    ) -> Self {
        Self {
            coordinator: hotkeys::CaptureCoordinator::new(
                hotkeys::HotkeyMode::PushToTalk,
                hotkeys::CaptureConfig::default(),
            ),
            recorder,
            history,
            target_resolver,
            active_target: None,
            processor,
            injector,
            unknown_field_policy: inject::UnknownFieldPolicy::default(),
            prefer_clipboard: false,
            default_cleanup_dial: CleanupDial::Light,
            active_shortcut_role: None,
            capture_cleanup_override: None,
            microphone_permission_ready_pending: false,
            first_dictation_completion_pending: false,
        }
    }

    #[cfg(test)]
    fn with_target_resolver(
        mut self,
        target_resolver: Box<dyn profiles::ResolveSessionTarget + Send>,
    ) -> Self {
        self.target_resolver = target_resolver;
        self
    }

    #[cfg(test)]
    fn with_processor(mut self, processor: Box<dyn pipeline::CaptureProcessor + Send>) -> Self {
        self.processor = processor;
        self
    }

    #[cfg(test)]
    fn with_injector(mut self, injector: Box<dyn inject::TextInjector + Send>) -> Self {
        self.injector = injector;
        self
    }

    fn set_hotkey_mode(
        &mut self,
        mode: settings::HotkeyModeSetting,
        capture: settings::CaptureSettings,
    ) -> Result<(), HotkeyModeUpdateError> {
        if !self.is_idle() {
            return Err(HotkeyModeUpdateError::CaptureActive);
        }
        self.coordinator = hotkey_coordinator_from_settings(mode, capture);
        Ok(())
    }

    fn set_default_cleanup_dial(
        &mut self,
        dial: CleanupDial,
    ) -> Result<(), CleanupDialUpdateError> {
        if !self.is_idle() {
            return Err(CleanupDialUpdateError::CaptureActive);
        }
        self.default_cleanup_dial = dial;
        Ok(())
    }

    fn set_asr_adapter_state(
        &mut self,
        state: engine::LocalAsrAdapterState,
    ) -> Result<Option<engine::EngineLane>, AsrRuntimeUpdateError> {
        if !self.is_idle() {
            return Err(AsrRuntimeUpdateError::CaptureActive);
        }
        let mut processor: Box<dyn pipeline::CaptureProcessor + Send> =
            Box::new(pipeline::default_runtime_pipeline_with_asr(state));
        let warmup_started = Instant::now();
        let warmed_lane = processor.warm_up()?;
        if let Some(lane) = warmed_lane {
            println!(
                "Kaydence ASR replacement warmed: lane={} elapsed_ms={}",
                engine_lane_snapshot_label(lane),
                elapsed_ms(warmup_started)
            );
        }
        self.processor = processor;
        Ok(warmed_lane)
    }

    fn warm_up_asr(&mut self) -> Result<Option<engine::EngineLane>, pipeline::PipelineError> {
        self.processor.warm_up()
    }

    fn is_idle(&self) -> bool {
        self.coordinator.state() == hotkeys::CaptureState::Idle
    }

    #[cfg(test)]
    fn take_first_dictation_completion(&mut self) -> bool {
        let completed = self.first_dictation_completion_pending;
        self.first_dictation_completion_pending = false;
        completed
    }

    fn take_first_run_proof(&mut self) -> HotkeyRuntimeFirstRunProof {
        let proof = HotkeyRuntimeFirstRunProof {
            microphone_permission_ready: self.microphone_permission_ready_pending,
            first_dictation_completed: self.first_dictation_completion_pending,
        };
        self.microphone_permission_ready_pending = false;
        self.first_dictation_completion_pending = false;
        proof
    }

    fn handle_signal(
        &mut self,
        signal: hotkeys::Signal,
    ) -> Result<Option<u64>, HotkeyRuntimeError> {
        self.handle_shortcut_signal(HotkeyShortcutRole::Primary, signal)
    }

    fn effective_cleanup_dial(&self, target: &profiles::SessionTarget) -> CleanupDial {
        let target_default = if target.profile.user_edited {
            target.profile.cleanup_dial
        } else {
            self.default_cleanup_dial
        };
        self.capture_cleanup_override.unwrap_or(target_default)
    }

    fn handle_shortcut_signal(
        &mut self,
        role: HotkeyShortcutRole,
        signal: hotkeys::Signal,
    ) -> Result<Option<u64>, HotkeyRuntimeError> {
        if !self.should_accept_shortcut_signal(role, signal) {
            return Ok(None);
        }

        let starts_new_capture = matches!(self.coordinator.state(), hotkeys::CaptureState::Idle)
            && matches!(signal, hotkeys::Signal::Press { .. });
        if starts_new_capture {
            self.active_shortcut_role = Some(role);
            self.capture_cleanup_override = match role {
                HotkeyShortcutRole::Primary => None,
                HotkeyShortcutRole::CleanupOverride => Some(CleanupDial::Raw),
            };
        }

        let was_finalizing = matches!(
            self.coordinator.state(),
            hotkeys::CaptureState::Finalizing { .. }
        );
        let at_ms = signal_at_ms(signal);
        let action = self.coordinator.step(signal);

        match action {
            hotkeys::Action::None => {}
            hotkeys::Action::StartCapture => {
                let id = match self.recorder.start_capture(at_ms) {
                    Ok(id) => id,
                    Err(err) => {
                        self.coordinator.reset();
                        self.active_shortcut_role = None;
                        self.capture_cleanup_override = None;
                        return Err(err.into());
                    }
                };
                let target = self.target_resolver.resolve_session_target();
                let started = events::SessionEvent::Started {
                    id,
                    target_app: target.app.clone(),
                    at_ms,
                };
                if let Err(err) = self.history.record_event(&started) {
                    let _ = self.recorder.discard_capture(at_ms);
                    self.coordinator.reset();
                    return Err(err.into());
                }
                self.active_target = Some(target.clone());
                println!(
                    "Kaydence capture started: id={:?} target_app={} profile={} sessions_dir={}",
                    id,
                    target.app.name,
                    target.profile.id,
                    self.recorder.sessions_dir().display()
                );
            }
            hotkeys::Action::FinalizeCapture => {
                let summary = self.recorder.finalize_capture(at_ms)?;
                if summary.samples_written > 0 {
                    self.microphone_permission_ready_pending = true;
                }
                let bound_target = self.active_target.take();
                if let Some(target) = &bound_target {
                    let cleanup_dial = self.effective_cleanup_dial(target);
                    self.processor.set_cleanup_dial(cleanup_dial);
                }
                let events = self.processor.process_capture(&summary)?;
                let committed = pipeline::committed_text(&events);
                self.history.record_events(&events)?;
                let injection_event = committed.map(|committed| match &bound_target {
                    Some(bound) => {
                        let current = self.target_resolver.resolve_session_target();
                        match inject::verify_focus_binding(
                            &focus_target(bound),
                            &focus_target(&current),
                        ) {
                            Ok(()) => inject::inject_committed_text(
                                self.injector.as_mut(),
                                committed.id,
                                &committed.text,
                                self.unknown_field_policy,
                                self.prefer_clipboard,
                            ),
                            Err(reason) => events::SessionEvent::Held {
                                id: committed.id,
                                reason,
                            },
                        }
                    }
                    None => events::SessionEvent::Held {
                        id: committed.id,
                        reason: events::HoldReason::FocusChanged,
                    },
                });
                if let Some(event) = injection_event {
                    let completed_dictation =
                        matches!(event, events::SessionEvent::Injected { .. });
                    self.history.record_event(&event)?;
                    if completed_dictation {
                        self.first_dictation_completion_pending = true;
                    }
                    println!("Kaydence injection outcome: event={event:?}");
                }
                println!(
                    "Kaydence capture processed: id={:?} events={} samples={} started_ms={} finalized_ms={}",
                    summary.id,
                    events.len(),
                    summary.samples_written,
                    summary.started_ms,
                    summary.finalized_ms
                );
                self.active_shortcut_role = None;
                self.capture_cleanup_override = None;
            }
            hotkeys::Action::DiscardCapture => {
                let discarded = self.recorder.discard_capture(at_ms)?;
                self.active_target = None;
                self.active_shortcut_role = None;
                self.capture_cleanup_override = None;
                self.history.delete_session(discarded.id)?;
                println!(
                    "Kaydence capture discarded: id={:?} path={} started_ms={} discarded_ms={} removed={}",
                    discarded.id,
                    discarded.wal_path.display(),
                    discarded.started_ms,
                    discarded.discarded_ms,
                    discarded.removed
                );
            }
        }

        if !was_finalizing {
            if let hotkeys::CaptureState::Finalizing { ends_ms, .. } = self.coordinator.state() {
                return Ok(Some(ends_ms));
            }
        }
        Ok(None)
    }

    /// Step 2 of `dispatch_hotkey_signal`, under the runtime lock: apply one
    /// hotkey edge from any source (plugin, Windows Right-Alt listener, Wayland
    /// control socket) and collect what the caller must do after releasing the
    /// lock: the tail-tick wake time and the first-run proof.
    fn apply_hotkey_signal(
        &mut self,
        role: HotkeyShortcutRole,
        signal: hotkeys::Signal,
    ) -> (Option<u64>, HotkeyRuntimeFirstRunProof) {
        match self.handle_shortcut_signal(role, signal) {
            Ok(tail_wake_ms) => (tail_wake_ms, self.take_first_run_proof()),
            Err(err) => {
                eprintln!("Kaydence hotkey runtime failed: {err}");
                (None, HotkeyRuntimeFirstRunProof::default())
            }
        }
    }

    fn should_accept_shortcut_signal(
        &self,
        role: HotkeyShortcutRole,
        signal: hotkeys::Signal,
    ) -> bool {
        if matches!(signal, hotkeys::Signal::Tick { .. }) {
            return true;
        }
        if matches!(self.coordinator.state(), hotkeys::CaptureState::Idle)
            && matches!(signal, hotkeys::Signal::Press { .. })
        {
            return true;
        }
        self.active_shortcut_role == Some(role)
    }
}

#[cfg(desktop)]
fn apply_selected_asr_to_runtime(
    snapshot: &settings::AppSnapshot,
    app_data_dir: &Path,
    runtime: &HotkeyRuntimeHandle,
) -> Result<Option<engine::EngineLane>, AsrRuntimeUpdateError> {
    runtime.apply_asr_adapter_state(selected_asr_runtime_state(
        &snapshot.settings.first_run,
        &models::source_tree_registry_path(),
        &app_data_dir.join("models"),
    ))
}

#[cfg(desktop)]
fn complete_asr_runtime_update(
    state: &RuntimeSnapshot,
    result: Result<Option<engine::EngineLane>, AsrRuntimeUpdateError>,
) -> Result<settings::AppSnapshot, String> {
    match result {
        Ok(Some(lane)) => state.mark_asr_adapter_ready(lane),
        Ok(None) => {}
        Err(err @ AsrRuntimeUpdateError::Warmup(_)) => {
            let detail = err.to_string();
            state.mark_asr_adapter_warmup_failed(&detail);
        }
        Err(err) => return Err(err.to_string()),
    }
    Ok(state.snapshot())
}

#[cfg(desktop)]
fn first_run_pending_asr_runtime_status(
    selected_model_id: Option<String>,
    lane: Option<String>,
) -> settings::FirstRunAsrRuntimeStatus {
    settings::FirstRunAsrRuntimeStatus {
        state: settings::FirstRunAsrRuntimeState::Pending,
        selected_model_id,
        lane,
        runtime: None,
        artifact_path: None,
        artifact_size_bytes: None,
        adapter_ready: false,
        detail: "Local ASR runtime is waiting for model artifact verification.".to_string(),
        proof_requirement:
            "Refresh model readiness and load a real ASR adapter before claiming transcript output."
                .to_string(),
    }
}

#[cfg(desktop)]
fn first_run_asr_runtime_status(
    state: &engine::LocalAsrAdapterState,
) -> settings::FirstRunAsrRuntimeStatus {
    match state {
        engine::LocalAsrAdapterState::Pending {
            selected_model_id,
            lane,
        } => settings::FirstRunAsrRuntimeStatus {
            state: settings::FirstRunAsrRuntimeState::Pending,
            selected_model_id: selected_model_id.clone(),
            lane: Some(engine_lane_snapshot_label(*lane).to_string()),
            runtime: None,
            artifact_path: None,
            artifact_size_bytes: None,
            adapter_ready: false,
            detail: selected_model_id
                .as_ref()
                .map(|model_id| {
                    format!(
                        "Local ASR runtime is waiting for a verified artifact for {model_id}."
                    )
                })
                .unwrap_or_else(|| {
                    "Local ASR runtime is waiting for a selected model.".to_string()
                }),
            proof_requirement:
                "A selected ASR model must verify under app-data models/ before inference can load."
                    .to_string(),
        },
        engine::LocalAsrAdapterState::Blocked {
            selected_model_id,
            lane,
            reason,
        } => settings::FirstRunAsrRuntimeStatus {
            state: settings::FirstRunAsrRuntimeState::Blocked,
            selected_model_id: selected_model_id.clone(),
            lane: Some(engine_lane_snapshot_label(*lane).to_string()),
            runtime: None,
            artifact_path: None,
            artifact_size_bytes: None,
            adapter_ready: false,
            detail: format!("Selected ASR runtime is blocked: {reason}"),
            proof_requirement:
                "Resolve the selected model artifact or registry error before claiming local ASR readiness."
                    .to_string(),
        },
        engine::LocalAsrAdapterState::VerifiedArtifact { spec } => {
            settings::FirstRunAsrRuntimeStatus {
                state: settings::FirstRunAsrRuntimeState::VerifiedArtifact,
                selected_model_id: Some(spec.model_id.clone()),
                lane: Some(engine_lane_snapshot_label(spec.lane).to_string()),
                runtime: Some(spec.runtime.clone()),
                artifact_path: Some(spec.artifact_path.display().to_string()),
                artifact_size_bytes: Some(spec.artifact_size_bytes),
                adapter_ready: false,
                detail: format!(
                    "Verified {} artifact is ready; adapter warmup has not completed in the active build.",
                    spec.runtime
                ),
                proof_requirement:
                    "Do not claim first-dictation readiness until a real adapter warms this artifact and passes the golden-clip latency gate."
                        .to_string(),
            }
        }
    }
}

#[cfg(desktop)]
fn engine_lane_snapshot_label(lane: engine::EngineLane) -> &'static str {
    match lane {
        engine::EngineLane::LocalCpu => "cpu",
        engine::EngineLane::LocalGpu => "gpu",
        engine::EngineLane::ByokCloud => "byok_cloud",
    }
}

#[cfg(desktop)]
fn selected_asr_runtime_state(
    first_run: &settings::FirstRunStatus,
    registry_path: &Path,
    models_dir: &Path,
) -> engine::LocalAsrAdapterState {
    let selected_model_id = first_run.selected_asr_model_id.clone();
    let fallback_lane = selected_model_id
        .as_deref()
        .and_then(|model_id| {
            first_run
                .asr_candidates
                .iter()
                .find(|candidate| candidate.id == model_id)
        })
        .and_then(|candidate| candidate.lane.as_deref())
        .map(|lane| local_engine_lane(Some(lane)))
        .unwrap_or(engine::EngineLane::LocalCpu);

    let Some(model_id) = selected_model_id.clone() else {
        return engine::LocalAsrAdapterState::Pending {
            selected_model_id: None,
            lane: fallback_lane,
        };
    };

    let registry = match models::ModelRegistry::load(registry_path) {
        Ok(registry) => registry,
        Err(err) => {
            return engine::LocalAsrAdapterState::Blocked {
                selected_model_id,
                lane: fallback_lane,
                reason: format!("model registry unavailable: {err}"),
            };
        }
    };
    let model = match registry.require(&model_id) {
        Ok(model) => model,
        Err(err) => {
            return engine::LocalAsrAdapterState::Blocked {
                selected_model_id,
                lane: fallback_lane,
                reason: err.to_string(),
            };
        }
    };
    let lane = effective_model_engine_lane(model);
    if model.task != models::ModelTask::Asr {
        return engine::LocalAsrAdapterState::Blocked {
            selected_model_id,
            lane,
            reason: format!("selected model task is {:?}", model.task),
        };
    }

    match model.verify_artifact(models_dir) {
        Ok(models::ModelArtifactStatus::Ready { path, size_bytes }) => {
            engine::LocalAsrAdapterState::VerifiedArtifact {
                spec: engine::LocalAsrAdapterSpec {
                    model_id,
                    lane,
                    runtime: model.runtime.clone(),
                    artifact_path: path,
                    artifact_size_bytes: size_bytes,
                },
            }
        }
        Ok(models::ModelArtifactStatus::Missing { path }) => {
            engine::LocalAsrAdapterState::Blocked {
                selected_model_id,
                lane,
                reason: format!("selected artifact is missing at {}", path.display()),
            }
        }
        Err(err) => engine::LocalAsrAdapterState::Blocked {
            selected_model_id,
            lane,
            reason: err.to_string(),
        },
    }
}

#[cfg(desktop)]
fn local_engine_lane(lane: Option<&str>) -> engine::EngineLane {
    match lane {
        Some("gpu") => engine::EngineLane::LocalGpu,
        Some("byok") | Some("cloud") => engine::EngineLane::ByokCloud,
        _ => engine::EngineLane::LocalCpu,
    }
}

#[cfg(desktop)]
fn effective_model_engine_lane(model: &models::ModelEntry) -> engine::EngineLane {
    let requested = local_engine_lane(model.lane.as_deref());
    if requested == engine::EngineLane::LocalGpu
        && model.runtime == "whisper.cpp"
        && !cfg!(all(target_os = "macos", feature = "asr-whisper-metal"))
    {
        engine::EngineLane::LocalCpu
    } else {
        requested
    }
}

#[cfg(desktop)]
fn effective_model_lane_label(model: &models::ModelEntry) -> Option<String> {
    model
        .lane
        .as_ref()
        .map(|_| engine_lane_snapshot_label(effective_model_engine_lane(model)).to_string())
}

#[cfg(desktop)]
fn focus_target(target: &profiles::SessionTarget) -> inject::FocusTarget {
    match target.source {
        profiles::TargetSource::Detected => inject::FocusTarget::detected(target.app.clone()),
        profiles::TargetSource::Unknown => inject::FocusTarget::unknown(target.app.clone()),
    }
}

#[cfg(desktop)]
fn signal_at_ms(signal: hotkeys::Signal) -> u64 {
    match signal {
        hotkeys::Signal::Press { at_ms }
        | hotkeys::Signal::Release { at_ms }
        | hotkeys::Signal::Tick { at_ms }
        | hotkeys::Signal::Chord { at_ms } => at_ms,
    }
}

/// Bindings the global-shortcut plugin cannot serve on this platform, owned by
/// a native listener instead: bare and Shift+Right-Alt on Windows, where
/// `RegisterHotKey` cannot deliver a lone modifier (ADR-0022).
#[cfg(desktop)]
fn served_by_native_listener(shortcut: &tauri_plugin_global_shortcut::Shortcut) -> bool {
    cfg!(target_os = "windows") && shortcut.key == tauri_plugin_global_shortcut::Code::AltRight
}

#[cfg(desktop)]
fn hotkey_coordinator_from_settings(
    mode: settings::HotkeyModeSetting,
    capture: settings::CaptureSettings,
) -> hotkeys::CaptureCoordinator {
    hotkeys::CaptureCoordinator::new(
        hotkey_mode_from_setting(mode),
        hotkeys::CaptureConfig {
            min_capture_ms: capture.min_capture_ms,
            tail_buffer_ms: capture.tail_buffer_ms,
            debounce_ms: capture.debounce_ms,
        },
    )
}

#[cfg(desktop)]
fn hotkey_mode_from_setting(mode: settings::HotkeyModeSetting) -> hotkeys::HotkeyMode {
    match mode {
        settings::HotkeyModeSetting::PushToTalk => hotkeys::HotkeyMode::PushToTalk,
        settings::HotkeyModeSetting::Toggle => hotkeys::HotkeyMode::Toggle,
    }
}

#[cfg(desktop)]
fn shortcut_from_binding(
    binding: &str,
) -> Result<tauri_plugin_global_shortcut::Shortcut, settings::SettingsError> {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

    match settings::normalize_hotkey_binding(binding)?.as_str() {
        settings::DEFAULT_HOTKEY_BINDING => Ok(Shortcut::new(None, Code::AltRight)),
        "F13" => Ok(Shortcut::new(None, Code::F13)),
        "F14" => Ok(Shortcut::new(None, Code::F14)),
        "Control+Space" => Ok(Shortcut::new(Some(Modifiers::CONTROL), Code::Space)),
        "Shift+F13" => Ok(Shortcut::new(Some(Modifiers::SHIFT), Code::F13)),
        other => Err(settings::SettingsError::InvalidHotkeyBinding(
            other.to_string(),
        )),
    }
}

#[cfg(desktop)]
fn cleanup_override_shortcut_from_binding(
    binding: &str,
) -> Result<tauri_plugin_global_shortcut::Shortcut, settings::SettingsError> {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

    let compact = binding
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-' && *ch != '_')
        .flat_map(char::to_lowercase)
        .collect::<String>();

    match compact.as_str() {
        "shift+rightalt" | "shiftrightalt" | "shift+rightoption" | "shiftrightoption" => {
            Ok(Shortcut::new(Some(Modifiers::SHIFT), Code::AltRight))
        }
        _ => Err(settings::SettingsError::InvalidHotkeyBinding(
            binding.trim().to_string(),
        )),
    }
}

#[cfg(desktop)]
fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(desktop)]
fn apply_hotkey_first_run_proof_to_snapshot(
    state: &RuntimeSnapshot,
    proof: HotkeyRuntimeFirstRunProof,
) {
    if proof.microphone_permission_ready {
        state.mark_microphone_permission_ready();
    }

    if proof.first_dictation_completed {
        if let Err(err) = state.mark_first_dictation_completed() {
            eprintln!("Kaydence first dictation completion save failed: {err}");
        }
    }
}

#[cfg(desktop)]
fn apply_platform_permission_proofs_to_snapshot(
    state: &RuntimeSnapshot,
    proofs: &[inject::PlatformPermissionProof],
) {
    for proof in proofs {
        state.mark_permission_requirement_ready(proof.requirement_id, proof.detail, proof.action);
    }
}

#[cfg(desktop)]
fn apply_hotkey_first_run_proof<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    proof: HotkeyRuntimeFirstRunProof,
) {
    apply_hotkey_first_run_proof_to_snapshot(&app.state::<RuntimeSnapshot>(), proof);
}

#[cfg(desktop)]
fn schedule_tail_tick(runtime: &Arc<Mutex<HotkeyRuntime>>, started: Instant, ends_ms: u64) {
    let runtime = Arc::clone(runtime);
    std::thread::spawn(move || {
        let sleep_ms = ends_ms.saturating_sub(elapsed_ms(started));
        if sleep_ms > 0 {
            std::thread::sleep(Duration::from_millis(sleep_ms));
        }
        let at_ms = elapsed_ms(started);
        match runtime.lock() {
            Ok(mut runtime) => {
                if let Err(err) = runtime.handle_signal(hotkeys::Signal::Tick { at_ms }) {
                    eprintln!("Kaydence hotkey tail tick failed: {err}");
                }
            }
            Err(_) => {
                eprintln!("Kaydence hotkey runtime lock poisoned during tail tick");
            }
        }
    });
}

/// The one hotkey dispatch path (ADR-0022 + ADR-0023). Every edge source runs
/// the same four steps in order:
///
/// 1. `mark_input_permission_ready`;
/// 2. runtime lock, then [`HotkeyRuntime::apply_hotkey_signal`] (under the lock);
/// 3. first-run proof;
/// 4. tail tick, whenever one is returned.
///
/// The global-shortcut plugin and the Windows Right-Alt listener call this
/// directly. The compositor control socket and its capture-limit timer keep
/// their own lock scope (they need the capture state before and after), and run
/// steps 1 and 3–4 through [`finish_hotkey_signal`]. A poisoned lock is logged,
/// never a panic.
#[cfg(desktop)]
fn dispatch_hotkey_signal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    runtime: &Arc<Mutex<HotkeyRuntime>>,
    started: Instant,
    role: HotkeyShortcutRole,
    signal: hotkeys::Signal,
) {
    use tauri::Manager;

    app.state::<RuntimeSnapshot>().mark_input_permission_ready();
    let outcome = match runtime.lock() {
        Ok(mut runtime) => runtime.apply_hotkey_signal(role, signal),
        Err(_) => {
            eprintln!("Kaydence hotkey runtime lock poisoned");
            (None, HotkeyRuntimeFirstRunProof::default())
        }
    };
    finish_hotkey_signal(app, runtime, started, outcome);
}

/// Steps 3–4 of [`dispatch_hotkey_signal`], shared by every hotkey source.
#[cfg(desktop)]
fn finish_hotkey_signal<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    runtime: &Arc<Mutex<HotkeyRuntime>>,
    started: Instant,
    (tail_wake_ms, proof): (Option<u64>, HotkeyRuntimeFirstRunProof),
) {
    apply_hotkey_first_run_proof(app, proof);
    if let Some(ends_ms) = tail_wake_ms {
        schedule_tail_tick(runtime, started, ends_ms);
    }
}

/// Feeds Windows Right-Alt Raw Input edges into the same runtime path as the
/// plugin (ADR-0022). The role is fixed at press time for the whole hold.
#[cfg(target_os = "windows")]
struct RightAltRuntimeSink {
    app: tauri::AppHandle,
    runtime: Arc<Mutex<HotkeyRuntime>>,
    started: Instant,
    hold_role: Mutex<Option<HotkeyShortcutRole>>,
}

#[cfg(target_os = "windows")]
impl hotkeys::windows::RightAltSink for RightAltRuntimeSink {
    fn owns(&self, shift: bool) -> bool {
        use tauri::Manager;

        self.app
            .state::<HotkeyRuntimeHandle>()
            .right_alt_role(shift)
            .is_some()
    }

    fn deliver(&self, edge: hotkeys::raw_key::RightAltEdge, at: Instant) {
        use hotkeys::raw_key::RightAltEdge;
        use tauri::Manager;

        let at_ms = at
            .saturating_duration_since(self.started)
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let mut hold_role = self
            .hold_role
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (role, signal) = match edge {
            RightAltEdge::Down { shift } => {
                *hold_role = self
                    .app
                    .state::<HotkeyRuntimeHandle>()
                    .right_alt_role(shift);
                (*hold_role, hotkeys::Signal::Press { at_ms })
            }
            RightAltEdge::Chord => (*hold_role, hotkeys::Signal::Chord { at_ms }),
            RightAltEdge::Up => (hold_role.take(), hotkeys::Signal::Release { at_ms }),
        };
        drop(hold_role);
        if let Some(role) = role {
            dispatch_hotkey_signal(&self.app, &self.runtime, self.started, role, signal);
        }
    }
}

#[cfg(desktop)]
fn install_global_hotkey(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use hotkeys::Signal;
    use tauri::Manager;
    use tauri_plugin_global_shortcut::ShortcutState;

    let app_data_dir = app.path().app_data_dir()?;
    let settings = app.state::<RuntimeSnapshot>().snapshot().settings;
    let shortcut = shortcut_from_binding(&settings.hotkey.primary_binding)?;
    let cleanup_override_shortcut = match cleanup_override_shortcut_from_binding(
        &settings.hotkey.secondary_dial_override_binding,
    ) {
        Ok(override_shortcut) if override_shortcut != shortcut => Some(override_shortcut),
        Ok(_) => {
            eprintln!(
                "Kaydence cleanup override hotkey disabled: override chord matches primary hotkey"
            );
            None
        }
        Err(err) => {
            eprintln!("Kaydence cleanup override hotkey disabled: {err}");
            None
        }
    };
    let mut runtime = HotkeyRuntime::new(app_data_dir, &settings)?;
    let warmup_started = Instant::now();
    match runtime.warm_up_asr() {
        Ok(Some(lane)) => {
            app.state::<RuntimeSnapshot>().mark_asr_adapter_ready(lane);
            println!(
                "Kaydence ASR startup warmup complete: lane={} elapsed_ms={}",
                engine_lane_snapshot_label(lane),
                elapsed_ms(warmup_started)
            );
        }
        Ok(None) => {}
        Err(err) => {
            app.state::<RuntimeSnapshot>()
                .mark_asr_adapter_warmup_failed(&err.to_string());
            eprintln!("Kaydence ASR startup warmup failed: {err}");
        }
    }
    let runtime = Arc::new(Mutex::new(runtime));
    let started = Instant::now();
    let handler_runtime = Arc::clone(&runtime);

    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, observed, event| {
                let shortcut_role = app.state::<HotkeyRuntimeHandle>().shortcut_role(observed);
                let Some(shortcut_role) = shortcut_role else {
                    return;
                };
                let at_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                let signal = match event.state() {
                    ShortcutState::Pressed => Signal::Press { at_ms },
                    ShortcutState::Released => Signal::Release { at_ms },
                };
                dispatch_hotkey_signal(app, &handler_runtime, started, shortcut_role, signal);
            })
            .build(),
    )?;

    let handle = app.state::<HotkeyRuntimeHandle>();
    handle.set_runtime(Arc::clone(&runtime));
    // Windows Right-Alt (ADR-0022): start the Raw Input listener before
    // registering, so `register_shortcut` reports its outcome and a failed
    // listener surfaces as a registration failure (hotkeys invariant 3).
    #[cfg(target_os = "windows")]
    handle.start_right_alt_listener(app.handle().clone(), Arc::clone(&runtime), started);
    // Wayland hotkey (ADR-0023): the compositor runs `<app> record …`, which
    // reaches this runtime over the local control socket (a no-op off Linux).
    // Started before the native grab so a Wayland session keeps a working path
    // even when the X11 grab is refused.
    install_control_socket(app.handle(), &runtime, started);
    let compositor = hotkeys::control::compositor_hotkey_status();
    match handle.register_shortcut(app.handle(), shortcut) {
        Ok(()) => handle.set_primary_shortcut(shortcut),
        // On Wayland a compositor binding replaces the grab entirely.
        Err(err) if compositor == hotkeys::control::CompositorHotkey::Bound => {
            eprintln!("Kaydence native hotkey grab unavailable ({err}); compositor binding active");
        }
        Err(err) => {
            return Err(
                match hotkeys::control::wayland_hotkey_guidance(compositor) {
                    Some(guidance) => format!("{err}. {guidance}").into(),
                    None => err.into(),
                },
            );
        }
    }
    if let Some(override_shortcut) = cleanup_override_shortcut {
        match handle.register_shortcut(app.handle(), override_shortcut) {
            Ok(()) => handle.set_cleanup_override_shortcut(Some(override_shortcut)),
            Err(err) => {
                eprintln!("Kaydence cleanup override hotkey registration failed: {err}");
                handle.set_cleanup_override_shortcut(None);
            }
        }
    } else {
        handle.set_cleanup_override_shortcut(None);
    }
    match hotkeys::control::wayland_hotkey_guidance(compositor) {
        // An X11 grab on a Wayland desktop only ever sees XWayland windows, so
        // it is not a working global hotkey: say so instead of going silently
        // dead (hotkeys invariant 3). The first `record` command received
        // flips this to registered.
        Some(guidance) => app
            .state::<RuntimeSnapshot>()
            .mark_hotkey_registration_failed(guidance),
        None => app.state::<RuntimeSnapshot>().mark_hotkey_registered(),
    }
    Ok(())
}

/// Serve the compositor control socket (Linux; a no-op elsewhere).
#[cfg(desktop)]
fn install_control_socket<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    runtime: &Arc<Mutex<HotkeyRuntime>>,
    started: Instant,
) {
    let app = app.clone();
    let runtime = Arc::clone(runtime);
    match hotkeys::control::serve(move |cmd| control_command(&app, &runtime, started, cmd)) {
        Ok(Some(path)) => println!("Kaydence control socket ready: {path}"),
        Ok(None) => {}
        Err(err) => eprintln!("Kaydence control socket unavailable: {err}"),
    }
}

/// One `record <verb>` from a compositor binding → the same hotkey edge the
/// native grab would produce, decided and applied under one runtime lock.
#[cfg(desktop)]
fn control_command<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    runtime: &Arc<Mutex<HotkeyRuntime>>,
    started: Instant,
    cmd: hotkeys::ControlCommand,
) -> Result<&'static str, &'static str> {
    let at_ms = elapsed_ms(started);
    if cmd != hotkeys::ControlCommand::Status {
        // Step 1. A command arrived: the compositor binding demonstrably works.
        let snapshot = app.state::<RuntimeSnapshot>();
        let first_run = snapshot.snapshot().settings.first_run;
        if !first_run.hotkey_registered {
            snapshot.mark_hotkey_registered();
        }
        if !first_run.input_permission_ready {
            snapshot.mark_input_permission_ready();
        }
    }
    let (outcome, label) = {
        let Ok(mut rt) = runtime.lock() else {
            return Err("runtime-unavailable");
        };
        let before = rt.coordinator.state();
        let outcome = hotkeys::control_signal(rt.coordinator.mode(), before, cmd, at_ms)
            .map(|signal| rt.apply_hotkey_signal(HotkeyShortcutRole::Primary, signal));
        let after = rt.coordinator.state();
        let capture_started_ms = match (before, after) {
            (hotkeys::CaptureState::Idle, hotkeys::CaptureState::Capturing { started_ms, .. }) => {
                Some(started_ms)
            }
            _ => None,
        };
        (
            outcome.map(|o| (o, capture_started_ms)),
            hotkeys::status_label(after),
        )
    };
    if let Some((step_outcome, capture_started_ms)) = outcome {
        finish_hotkey_signal(app, runtime, started, step_outcome);
        if let Some(capture_started_ms) = capture_started_ms {
            schedule_control_capture_limit(app.clone(), runtime, started, capture_started_ms);
        }
    }
    Ok(label)
}

/// Safety stop for a socket-started capture whose release never arrived.
/// Fires only if that exact capture is still running.
#[cfg(desktop)]
fn schedule_control_capture_limit<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    runtime: &Arc<Mutex<HotkeyRuntime>>,
    started: Instant,
    capture_started_ms: u64,
) {
    let runtime = Arc::clone(runtime);
    std::thread::spawn(move || {
        use tauri::Manager;

        std::thread::sleep(Duration::from_millis(
            hotkeys::control::CONTROL_CAPTURE_LIMIT_MS,
        ));
        app.state::<RuntimeSnapshot>().mark_input_permission_ready();
        let outcome = match runtime.lock() {
            Ok(mut rt) => match rt.coordinator.state() {
                state @ hotkeys::CaptureState::Capturing { started_ms, .. }
                    if started_ms == capture_started_ms =>
                {
                    eprintln!("Kaydence control capture reached the 5-minute safety stop");
                    hotkeys::control_signal(
                        rt.coordinator.mode(),
                        state,
                        hotkeys::ControlCommand::Stop,
                        elapsed_ms(started),
                    )
                    .map(|signal| rt.apply_hotkey_signal(HotkeyShortcutRole::Primary, signal))
                }
                _ => None,
            },
            Err(_) => None,
        };
        if let Some(step_outcome) = outcome {
            finish_hotkey_signal(&app, &runtime, started, step_outcome);
        }
    });
}

#[cfg(desktop)]
const MAIN_WINDOW_LABEL: &str = "main";
#[cfg(desktop)]
const TRAY_OPEN_ID: &str = "tray-open";
#[cfg(desktop)]
const TRAY_QUIT_ID: &str = "tray-quit";

#[cfg(desktop)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainWindowOpenAction {
    FocusExisting,
    CreateFromConfig,
}

#[cfg(desktop)]
#[derive(Debug, Default)]
struct DesktopLifecycle {
    window_close_pending: AtomicBool,
}

#[cfg(desktop)]
impl DesktopLifecycle {
    fn mark_window_close(&self) {
        self.window_close_pending.store(true, Ordering::SeqCst);
    }

    fn cancel_window_close(&self) {
        self.window_close_pending.store(false, Ordering::SeqCst);
    }

    fn should_keep_running_after_exit_request(&self, code: Option<i32>) -> bool {
        self.window_close_pending.swap(false, Ordering::SeqCst) && code.is_none()
    }
}

#[cfg(desktop)]
fn main_window_open_action(window_exists: bool) -> MainWindowOpenAction {
    if window_exists {
        MainWindowOpenAction::FocusExisting
    } else {
        MainWindowOpenAction::CreateFromConfig
    }
}

/// Carry out a user close of the cockpit: destroy the WebView, keep the
/// tray-owned runtime (AGENTS.md invariant 7). Called from the app-level
/// `RunEvent` callback, never a `Builder::on_window_event` listener: Tauri
/// attaches those asynchronously (a posted `AddEventListener` message), so a
/// close in the first few hundred ms after the window appears bypassed them
/// and the whole process exited (ADR-0021). The app callback is live from the
/// first event and runs before Tauri checks for `prevent_close`.
#[cfg(desktop)]
fn intercept_main_window_close(
    app: &tauri::AppHandle,
    lifecycle: &DesktopLifecycle,
    api: &tauri::CloseRequestApi,
) {
    lifecycle.mark_window_close();
    // Not yet registered with the manager: Tauri's default close destroys the
    // window, and the pending flag still keeps the tray runtime alive.
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    api.prevent_close();
    if let Err(err) = window.destroy() {
        lifecycle.cancel_window_close();
        eprintln!("{} window close failed: {err}", settings::APP_NAME);
    }
}

#[cfg(desktop)]
fn open_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    match main_window_open_action(app.get_webview_window(MAIN_WINDOW_LABEL).is_some()) {
        MainWindowOpenAction::FocusExisting => {
            let window = app
                .get_webview_window(MAIN_WINDOW_LABEL)
                .ok_or_else(|| "Main window disappeared before it could be focused".to_string())?;
            window.unminimize().map_err(|err| err.to_string())?;
            window.show().map_err(|err| err.to_string())?;
            window.set_focus().map_err(|err| err.to_string())
        }
        MainWindowOpenAction::CreateFromConfig => {
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == MAIN_WINDOW_LABEL)
                .ok_or_else(|| "Main window configuration is missing".to_string())?;
            tauri::WebviewWindowBuilder::from_config(app, config)
                .map_err(|err| err.to_string())?
                .build()
                .map(|_| ())
                .map_err(|err| err.to_string())
        }
    }
}

#[cfg(desktop)]
fn install_tray(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let open = MenuItem::with_id(
        app,
        TRAY_OPEN_ID,
        format!("Open {}", settings::APP_NAME),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        TRAY_QUIT_ID,
        format!("Quit {}", settings::APP_NAME),
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(settings::APP_NAME);
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

/// Run the Kaydence desktop app. Called by the thin `main.rs` binary.
///
/// Run the Tauri shell plus the current hotkey/audio/pipeline runtime.
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_dialog::init());

    #[cfg(desktop)]
    let lifecycle = DesktopLifecycle::default();
    #[cfg(desktop)]
    let builder = builder
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_OPEN_ID => {
                if let Err(err) = open_main_window(app) {
                    eprintln!("{} window open failed: {err}", settings::APP_NAME);
                }
            }
            TRAY_QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|app, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};

            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                if let Err(err) = open_main_window(app) {
                    eprintln!("{} window open failed: {err}", settings::APP_NAME);
                }
            }
        });

    let app = builder
        .manage(RuntimeSnapshot::default())
        .manage(HotkeyRuntimeHandle::default())
        .invoke_handler(tauri::generate_handler![
            app_snapshot,
            select_asr_model,
            set_hotkey_mode,
            set_hotkey_binding,
            set_cleanup_dial,
            refresh_model_readiness,
            install_model_artifact,
            first_run_model_download_preflight,
            first_run_model_download,
            first_run_permission_action,
            open_first_run_permission_settings,
            refresh_first_run_runtime_proofs,
            export_first_run_proof_plan,
            recent_history,
            delete_history_session,
            export_history_session,
            play_history_audio,
            purge_history
        ])
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri::Manager;

                let snapshot = app.state::<RuntimeSnapshot>();
                match app.path().app_data_dir() {
                    Ok(app_data_dir) => {
                        if let Err(err) = snapshot.refresh_models_from_app_data(&app_data_dir) {
                            eprintln!("Kaydence settings load failed: {err}");
                        }
                    }
                    Err(err) => snapshot.mark_model_readiness_failed(format!(
                        "App data directory unavailable: {err}"
                    )),
                }

                if let Err(err) = install_global_hotkey(app) {
                    app.state::<RuntimeSnapshot>()
                        .mark_hotkey_registration_failed(err.to_string());
                    eprintln!("Kaydence global hotkey disabled: {err}");
                }

                install_tray(app)?;
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|err| {
            eprintln!("{} failed to start: {err}", settings::APP_NAME);
            std::process::exit(1);
        });

    #[cfg(desktop)]
    app.run(move |app, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == MAIN_WINDOW_LABEL => {
            intercept_main_window_close(app, &lifecycle, &api);
        }
        tauri::RunEvent::ExitRequested { code, api, .. }
            if lifecycle.should_keep_running_after_exit_request(code) =>
        {
            api.prevent_exit();
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => {
            if let Err(err) = open_main_window(app) {
                eprintln!("{} window reopen failed: {err}", settings::APP_NAME);
            }
        }
        _ => {}
    });

    #[cfg(not(desktop))]
    app.run(|_, _| {});
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;
    use crate::events::{CleanupDial, HoldReason, InjectMethod};
    use sha2::{Digest, Sha256};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[test]
    fn user_window_close_is_a_one_shot_background_transition() {
        let lifecycle = DesktopLifecycle::default();

        lifecycle.mark_window_close();
        assert!(lifecycle.should_keep_running_after_exit_request(None));
        assert!(!lifecycle.should_keep_running_after_exit_request(None));
    }

    #[test]
    fn explicit_quit_and_failed_close_are_not_intercepted() {
        let lifecycle = DesktopLifecycle::default();

        lifecycle.mark_window_close();
        assert!(!lifecycle.should_keep_running_after_exit_request(Some(0)));

        lifecycle.mark_window_close();
        lifecycle.cancel_window_close();
        assert!(!lifecycle.should_keep_running_after_exit_request(None));
    }

    #[test]
    fn open_action_recreates_only_a_missing_cockpit() {
        assert_eq!(
            main_window_open_action(true),
            MainWindowOpenAction::FocusExisting
        );
        assert_eq!(
            main_window_open_action(false),
            MainWindowOpenAction::CreateFromConfig
        );
    }

    fn tmp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kaydence-hotkey-runtime-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn detected_app() -> events::AppRef {
        events::AppRef {
            id: "com.example.editor".to_string(),
            name: "Example Editor".to_string(),
        }
    }

    fn other_app() -> events::AppRef {
        events::AppRef {
            id: "com.example.mail".to_string(),
            name: "Example Mail".to_string(),
        }
    }

    fn session_target(app: events::AppRef, cleanup_dial: CleanupDial) -> profiles::SessionTarget {
        profiles::SessionTarget {
            app: app.clone(),
            profile: profiles::AppProfile::user_edited(
                "test-profile",
                "Test Profile",
                vec![app.id.clone()],
                cleanup_dial,
            ),
            source: profiles::TargetSource::Detected,
        }
    }

    struct QueueTargetResolver {
        targets: VecDeque<profiles::SessionTarget>,
        last: profiles::SessionTarget,
    }

    impl QueueTargetResolver {
        fn new(targets: Vec<profiles::SessionTarget>) -> Self {
            let last = targets
                .last()
                .expect("queue target resolver needs at least one target")
                .clone();
            Self {
                targets: targets.into(),
                last,
            }
        }
    }

    impl profiles::ResolveSessionTarget for QueueTargetResolver {
        fn resolve_session_target(&mut self) -> profiles::SessionTarget {
            if let Some(target) = self.targets.pop_front() {
                self.last = target.clone();
                target
            } else {
                self.last.clone()
            }
        }
    }

    struct ScriptedProcessor {
        cleanup_dial: CleanupDial,
        seen_dials: Arc<Mutex<Vec<CleanupDial>>>,
    }

    struct WarmupProcessor {
        calls: Arc<Mutex<u32>>,
    }

    impl ScriptedProcessor {
        fn new(seen_dials: Arc<Mutex<Vec<CleanupDial>>>) -> Self {
            Self {
                cleanup_dial: CleanupDial::Light,
                seen_dials,
            }
        }
    }

    impl pipeline::CaptureProcessor for ScriptedProcessor {
        fn set_cleanup_dial(&mut self, cleanup_dial: CleanupDial) {
            self.cleanup_dial = cleanup_dial;
            self.seen_dials.lock().unwrap().push(cleanup_dial);
        }

        fn process_capture(
            &mut self,
            summary: &audio::CaptureSessionSummary,
        ) -> Result<Vec<events::SessionEvent>, pipeline::PipelineError> {
            let mut events = vec![
                summary.audio_persisted_event(),
                events::SessionEvent::RawFinal {
                    id: summary.id,
                    text: "um hello captain".to_string(),
                },
            ];
            if self.cleanup_dial != CleanupDial::Raw {
                events.push(events::SessionEvent::CleanFinal {
                    id: summary.id,
                    text: "Hello captain.".to_string(),
                    dial: self.cleanup_dial,
                });
            }
            Ok(events)
        }
    }

    impl pipeline::CaptureProcessor for WarmupProcessor {
        fn warm_up(&mut self) -> Result<Option<engine::EngineLane>, pipeline::PipelineError> {
            *self.calls.lock().unwrap() += 1;
            Ok(Some(engine::EngineLane::LocalGpu))
        }

        fn set_cleanup_dial(&mut self, _cleanup_dial: CleanupDial) {}

        fn process_capture(
            &mut self,
            summary: &audio::CaptureSessionSummary,
        ) -> Result<Vec<events::SessionEvent>, pipeline::PipelineError> {
            Ok(vec![summary.audio_persisted_event()])
        }
    }

    struct TestInjector {
        caps: inject::InjectorCaps,
        field: inject::FieldKind,
        delivered: Arc<Mutex<Vec<String>>>,
    }

    impl TestInjector {
        fn no_target() -> Self {
            Self {
                caps: inject::InjectorCaps {
                    native_text_insert: false,
                    keystroke: inject::KeystrokeChannel::None,
                    clipboard: false,
                },
                field: inject::FieldKind::NoTarget,
                delivered: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn native(delivered: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                caps: inject::InjectorCaps {
                    native_text_insert: true,
                    keystroke: inject::KeystrokeChannel::None,
                    clipboard: false,
                },
                field: inject::FieldKind::Editable,
                delivered,
            }
        }
    }

    impl inject::TextInjector for TestInjector {
        fn caps(&self) -> inject::InjectorCaps {
            self.caps
        }

        fn focused_field(&self) -> inject::FieldKind {
            self.field
        }

        fn insert_native(&mut self, text: &str) -> Result<(), inject::InjectError> {
            self.delivered.lock().unwrap().push(text.to_string());
            Ok(())
        }

        fn synth_text(&mut self, text: &str) -> Result<(), inject::InjectError> {
            self.delivered.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }

    fn matching_profiles(cleanup_dial: CleanupDial) -> profiles::ProfileStore {
        let app = detected_app();
        profiles::ProfileStore::with_profiles(vec![profiles::AppProfile::user_edited(
            "example-editor",
            "Example Editor",
            vec![app.id],
            cleanup_dial,
        )])
    }

    fn sha256_for(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn first_run_registry_json(asr_hash: &str, vad_hash: &str) -> String {
        format!(
            r#"{{
              "schema_version": 1,
              "models": [
                {{
                  "id": "fixture-asr",
                  "task": "asr",
                  "lane": "cpu",
                  "runtime": "onnxruntime",
                  "file": "fixture-asr.onnx",
                  "sha256": "{asr_hash}",
                  "size_mb": 1,
                  "license": "Apache-2.0",
                  "min_hw": "any",
                  "recommended": true,
                  "sources": ["https\u003A//models.example.test/fixture-asr.onnx"]
                }},
                {{
                  "id": "fixture-vad",
                  "task": "vad",
                  "runtime": "onnxruntime",
                  "file": "fixture-vad.onnx",
                  "sha256": "{vad_hash}",
                  "size_mb": 1,
                  "license": "MIT",
                  "min_hw": "any",
                  "recommended": true,
                  "sources": ["https\u003A//models.example.test/fixture-vad.onnx"]
                }}
              ]
            }}"#
        )
    }

    fn selectable_asr_registry_json() -> String {
        r#"{
          "schema_version": 1,
          "models": [
            {
              "id": "fixture-asr",
              "task": "asr",
              "lane": "cpu",
              "runtime": "onnxruntime",
              "file": "fixture-asr.onnx",
              "sha256": "TODO",
              "size_mb": 1,
              "license": "Apache-2.0",
              "min_hw": "any",
              "recommended": true
            },
            {
              "id": "fixture-gpu",
              "task": "asr",
              "lane": "gpu",
              "runtime": "whisper.cpp",
              "file": "fixture-gpu.bin",
              "sha256": "TODO",
              "size_mb": 2,
              "license": "MIT",
              "min_hw": "metal_or_dgpu",
              "recommended": true
            },
            {
              "id": "fixture-vad",
              "task": "vad",
              "runtime": "onnxruntime",
              "file": "fixture-vad.onnx",
              "sha256": "TODO",
              "size_mb": 1,
              "license": "MIT",
              "min_hw": "any",
              "recommended": true
            }
          ]
        }"#
        .to_string()
    }

    fn write_first_run_registry(
        app_data: &std::path::Path,
        asr_hash: &str,
        vad_hash: &str,
    ) -> std::path::PathBuf {
        let registry_dir = app_data.join("registry");
        std::fs::create_dir_all(&registry_dir).unwrap();
        let registry_path = registry_dir.join("registry.json");
        std::fs::write(&registry_path, first_run_registry_json(asr_hash, vad_hash)).unwrap();
        registry_path
    }

    fn write_selectable_asr_registry(app_data: &std::path::Path) -> std::path::PathBuf {
        let registry_dir = app_data.join("registry");
        std::fs::create_dir_all(&registry_dir).unwrap();
        let registry_path = registry_dir.join("registry.json");
        std::fs::write(&registry_path, selectable_asr_registry_json()).unwrap();
        registry_path
    }

    fn write_ready_selectable_asr_registry(app_data: &std::path::Path) -> std::path::PathBuf {
        let mut registry: serde_json::Value =
            serde_json::from_str(&selectable_asr_registry_json()).unwrap();
        for model in registry["models"].as_array_mut().unwrap() {
            model["sha256"] = match model["id"].as_str().unwrap() {
                "fixture-asr" => sha256_for(b"cpu").into(),
                "fixture-gpu" => sha256_for(b"gpu").into(),
                "fixture-vad" => sha256_for(b"vad").into(),
                id => panic!("unexpected fixture model: {id}"),
            };
        }
        let registry_dir = app_data.join("registry");
        std::fs::create_dir_all(&registry_dir).unwrap();
        let registry_path = registry_dir.join("registry.json");
        std::fs::write(
            &registry_path,
            serde_json::to_vec_pretty(&registry).unwrap(),
        )
        .unwrap();
        registry_path
    }

    #[test]
    fn runtime_snapshot_reflects_hotkey_registration_success() {
        let state = RuntimeSnapshot::default();

        assert!(!state.snapshot().settings.first_run.hotkey_registered);
        state.mark_hotkey_registered();

        let snapshot = state.snapshot();
        assert!(snapshot.settings.first_run.hotkey_registered);
        assert_eq!(snapshot.settings.first_run.hotkey_registration_error, None);
    }

    #[test]
    fn runtime_snapshot_marks_input_permission_ready_from_hotkey_event() {
        let state = RuntimeSnapshot::default();

        assert!(!state.snapshot().settings.first_run.input_permission_ready);
        state.mark_input_permission_ready();

        let first_run = state.snapshot().settings.first_run;
        assert!(first_run.input_permission_ready);
        if cfg!(target_os = "macos") {
            let input_monitoring = first_run
                .permission_requirements
                .iter()
                .find(|requirement| requirement.id == "input_monitoring")
                .unwrap();
            assert_eq!(
                input_monitoring.state,
                settings::FirstRunPermissionState::Ready
            );
            assert!(input_monitoring.detail.contains("OS event stream"));
        }
    }

    #[test]
    fn runtime_snapshot_marks_microphone_permission_ready_from_audio_proof() {
        let state = RuntimeSnapshot::default();

        assert!(
            !state
                .snapshot()
                .settings
                .first_run
                .microphone_permission_ready
        );
        state.mark_microphone_permission_ready();

        let first_run = state.snapshot().settings.first_run;
        assert!(first_run.microphone_permission_ready);
        let microphone = first_run
            .permission_requirements
            .iter()
            .find(|requirement| requirement.id == "microphone")
            .unwrap();
        assert_eq!(microphone.state, settings::FirstRunPermissionState::Ready);
        assert!(microphone.detail.contains("persisted local audio"));
    }

    #[test]
    fn runtime_snapshot_marks_single_permission_requirement_from_platform_proof() {
        let state = RuntimeSnapshot::default();

        let marked = state.mark_permission_requirement_ready(
            "accessibility",
            "Runtime proof observed: macOS Accessibility preflight trusts this Kaydence process.",
            "No action needed; Accessibility proof is recorded for this runtime.",
        );

        if cfg!(target_os = "macos") {
            assert!(marked);
            let first_run = state.snapshot().settings.first_run;
            let accessibility = first_run
                .permission_requirements
                .iter()
                .find(|requirement| requirement.id == "accessibility")
                .unwrap();
            assert_eq!(
                accessibility.state,
                settings::FirstRunPermissionState::Ready
            );
            assert!(accessibility.detail.contains("Accessibility preflight"));
            let microphone = first_run
                .permission_requirements
                .iter()
                .find(|requirement| requirement.id == "microphone")
                .unwrap();
            assert_ne!(microphone.state, settings::FirstRunPermissionState::Ready);
            assert!(!first_run.ready_to_dictate());
        } else {
            assert!(!marked);
        }
    }

    #[test]
    fn platform_permission_proofs_update_only_reported_requirements() {
        let state = RuntimeSnapshot::default();

        apply_platform_permission_proofs_to_snapshot(
            &state,
            &[inject::PlatformPermissionProof {
                requirement_id: "accessibility",
                detail: "Runtime proof observed: test platform proof.",
                action: "No action needed; test proof recorded.",
            }],
        );

        if cfg!(target_os = "macos") {
            let first_run = state.snapshot().settings.first_run;
            let accessibility = first_run
                .permission_requirements
                .iter()
                .find(|requirement| requirement.id == "accessibility")
                .unwrap();
            assert_eq!(
                accessibility.state,
                settings::FirstRunPermissionState::Ready
            );
            assert!(accessibility.detail.contains("test platform proof"));
            assert!(!first_run.input_permission_ready);
            assert!(!first_run.microphone_permission_ready);
        }
    }

    #[test]
    fn platform_permission_input_monitoring_proof_updates_input_readiness_only() {
        let state = RuntimeSnapshot::default();

        apply_platform_permission_proofs_to_snapshot(
            &state,
            &[inject::PlatformPermissionProof {
                requirement_id: "input_monitoring",
                detail: "Runtime proof observed: macOS Input Monitoring preflight grants listen-event access.",
                action: "No action needed; Input Monitoring proof is recorded.",
            }],
        );

        if cfg!(target_os = "macos") {
            let first_run = state.snapshot().settings.first_run;
            let input_monitoring = first_run
                .permission_requirements
                .iter()
                .find(|requirement| requirement.id == "input_monitoring")
                .unwrap();
            assert_eq!(
                input_monitoring.state,
                settings::FirstRunPermissionState::Ready
            );
            assert!(input_monitoring
                .detail
                .contains("Input Monitoring preflight"));
            assert!(first_run.input_permission_ready);
            assert!(!first_run.microphone_permission_ready);
            assert!(!first_run.ready_to_dictate());
        } else {
            let first_run = state.snapshot().settings.first_run;
            assert!(!first_run.input_permission_ready);
            assert!(!first_run.microphone_permission_ready);
        }
    }

    #[test]
    fn runtime_snapshot_records_hotkey_registration_failure() {
        let state = RuntimeSnapshot::default();
        state.mark_hotkey_registered();
        state.mark_hotkey_registration_failed("shortcut already registered".to_string());

        let snapshot = state.snapshot();
        assert!(!snapshot.settings.first_run.hotkey_registered);
        assert_eq!(
            snapshot.settings.first_run.hotkey_registration_error,
            Some("shortcut already registered".to_string())
        );
    }

    #[test]
    fn runtime_snapshot_persists_first_dictation_completion() {
        let app_data = tmp();
        let state = RuntimeSnapshot::default();
        state.set_settings_store(&app_data);

        let snapshot = state.mark_first_dictation_completed().unwrap();

        assert!(snapshot.settings.first_run.first_dictation_completed);
        assert!(snapshot.settings.first_run.microphone_permission_ready);
        assert!(snapshot.settings.first_run.input_permission_ready);
        assert!(
            settings::SettingsStore::new(&app_data)
                .load()
                .unwrap()
                .first_dictation_completed
        );
        assert!(settings::SettingsStore::new(&app_data)
            .load()
            .unwrap()
            .first_dictation_completed_at_ms
            .is_some());

        let rehydrated = RuntimeSnapshot::default();
        rehydrated.set_settings_store(&app_data);
        rehydrated.apply_persisted_user_settings().unwrap();
        assert!(
            rehydrated
                .snapshot()
                .settings
                .first_run
                .first_dictation_completed
        );
        assert!(rehydrated
            .snapshot()
            .settings
            .first_run
            .setup_timing
            .completed_at_ms
            .is_some());
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn runtime_snapshot_starts_first_run_setup_timer_once() {
        let app_data = tmp();
        let state = RuntimeSnapshot::default();
        state.set_settings_store(&app_data);

        state.ensure_first_run_started_at(1_000).unwrap();
        state.ensure_first_run_started_at(2_000).unwrap();

        let persisted = settings::SettingsStore::new(&app_data).load().unwrap();
        assert_eq!(persisted.first_run_started_at_ms, Some(1_000));
        assert_eq!(
            state
                .snapshot()
                .settings
                .first_run
                .setup_timing
                .started_at_ms,
            Some(1_000)
        );
        assert_eq!(
            state.snapshot().settings.first_run.setup_timing.target_ms,
            settings::FIRST_RUN_SETUP_TARGET_MS
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn runtime_snapshot_times_first_dictation_completion_against_sixty_seconds() {
        let app_data = tmp();
        let state = RuntimeSnapshot::default();
        state.set_settings_store(&app_data);
        state.ensure_first_run_started_at(1_000).unwrap();

        let snapshot = state.mark_first_dictation_completed_at(60_500).unwrap();
        let timing = snapshot.settings.first_run.setup_timing;

        assert_eq!(timing.started_at_ms, Some(1_000));
        assert_eq!(timing.completed_at_ms, Some(60_500));
        assert_eq!(timing.elapsed_ms, Some(59_500));
        assert_eq!(timing.within_target, Some(true));

        let persisted = settings::SettingsStore::new(&app_data).load().unwrap();
        assert_eq!(persisted.first_run_started_at_ms, Some(1_000));
        assert_eq!(persisted.first_dictation_completed_at_ms, Some(60_500));
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn first_run_proof_export_writes_json_under_app_data() {
        let app_data = tmp();
        let state = RuntimeSnapshot::default();
        state.mark_hotkey_registered();

        let outcome =
            export_first_run_proof_plan_to_app_data(&state.snapshot(), &app_data, 42_4242).unwrap();

        assert!(outcome.exported);
        assert!(outcome.item_count >= 3);
        let json_path = std::path::PathBuf::from(outcome.json_path.unwrap());
        assert_eq!(
            json_path,
            app_data
                .join(history::EXPORTS_DIR)
                .join("first-run-proof-plan.json")
        );
        assert!(json_path.exists());

        let exported: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(
            exported["schema_version"].as_u64(),
            Some(u64::from(settings::FIRST_RUN_PROOF_PLAN_SCHEMA_VERSION))
        );
        assert_eq!(exported["generated_at_ms"].as_u64(), Some(42_4242));
        assert_eq!(exported["app_name"].as_str(), Some(settings::APP_NAME));
        assert!(exported["next_step"]["kind"].is_string());
        assert!(exported["asr_runtime"]["state"].is_string());
        assert_eq!(
            exported["proof_items"].as_array().unwrap().len(),
            outcome.item_count
        );
        assert!(exported["proof_items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "next_step"));
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn runtime_snapshot_blocks_placeholder_model_checksums() {
        let app_data = tmp();
        let registry_path = write_first_run_registry(&app_data, "TODO", "TODO");
        let state = RuntimeSnapshot::default();

        state.refresh_model_readiness(&registry_path, &app_data.join("models"));

        let first_run = state.snapshot().settings.first_run;
        assert!(!first_run.model_ready);
        assert_eq!(
            first_run.model_readiness_error,
            Some(
                "Model registry needs verified metadata, checksums, or artifact repair".to_string()
            )
        );
        assert_eq!(first_run.required_models.len(), 1);
        assert!(first_run
            .required_models
            .iter()
            .all(|model| model.state == settings::FirstRunModelState::Blocked));
        assert_eq!(
            first_run.recommended_asr_model_id.as_deref(),
            Some("fixture-asr")
        );
        assert_eq!(
            first_run.selected_asr_model_id.as_deref(),
            Some("fixture-asr")
        );
        assert_eq!(first_run.asr_candidates.len(), 1);
        assert!(first_run.asr_candidates[0].selected);
        assert_eq!(
            first_run.asr_candidates[0].recommendation.as_deref(),
            Some("CPU-safe first-run default")
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn runtime_snapshot_reports_missing_first_run_models() {
        let app_data = tmp();
        let registry_path =
            write_first_run_registry(&app_data, &sha256_for(b"asr"), &sha256_for(b"vad"));
        let state = RuntimeSnapshot::default();

        state.refresh_model_readiness(&registry_path, &app_data.join("models"));

        let first_run = state.snapshot().settings.first_run;
        assert!(!first_run.model_ready);
        assert_eq!(first_run.model_readiness_error, None);
        assert!(first_run
            .required_models
            .iter()
            .all(|model| model.state == settings::FirstRunModelState::Missing));
        assert!(first_run
            .required_models
            .iter()
            .all(|model| model.download_available
                && model.download_size_mb == Some(1)
                && model.download_source_count == 1));
        assert!(first_run
            .asr_candidates
            .iter()
            .all(|model| model.state == settings::FirstRunModelState::Missing));
        assert!(first_run
            .asr_candidates
            .iter()
            .all(|model| model.download_available
                && model.download_size_mb == Some(1)
                && model.download_source_count == 1));
        assert_eq!(
            first_run.asr_runtime.state,
            settings::FirstRunAsrRuntimeState::Blocked
        );
        assert_eq!(
            first_run.asr_runtime.selected_model_id.as_deref(),
            Some("fixture-asr")
        );
        assert!(first_run
            .asr_runtime
            .detail
            .contains("selected artifact is missing"));
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn model_download_preflight_exposes_reviewed_plan_without_fetching() {
        let app_data = tmp();
        let registry_path =
            write_first_run_registry(&app_data, &sha256_for(b"asr"), &sha256_for(b"vad"));
        let state = RuntimeSnapshot::default();
        state.refresh_model_readiness(&registry_path, &app_data.join("models"));

        let preflight = state
            .model_download_preflight(&registry_path, &app_data, "fixture-asr")
            .unwrap();

        assert_eq!(preflight.model_id, "fixture-asr");
        assert_eq!(preflight.state, settings::FirstRunModelState::Missing);
        assert!(preflight.available);
        assert_eq!(preflight.size_mb, Some(1));
        assert_eq!(preflight.source_count, 1);
        let expected_destination = app_data.join("models").join("fixture-asr.onnx");
        assert_eq!(
            preflight.destination_path.as_deref().map(PathBuf::from),
            Some(expected_destination.clone())
        );
        let expected_hash = sha256_for(b"asr");
        assert_eq!(
            preflight.expected_sha256.as_deref(),
            Some(expected_hash.as_str())
        );
        assert_eq!(
            preflight.sources,
            vec![format!(
                "{}models.example.test/fixture-asr.onnx",
                concat!("https", "://")
            )]
        );
        assert!(preflight.blocked_reason.is_none());
        assert!(preflight
            .proof_requirement
            .contains("post-fetch verification"));
        assert!(!expected_destination.exists());
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn model_download_preflight_blocks_placeholder_metadata() {
        let app_data = tmp();
        let registry_path = write_first_run_registry(&app_data, "TODO", "TODO");
        let state = RuntimeSnapshot::default();
        state.refresh_model_readiness(&registry_path, &app_data.join("models"));

        let preflight = state
            .model_download_preflight(&registry_path, &app_data, "fixture-asr")
            .unwrap();

        assert_eq!(preflight.model_id, "fixture-asr");
        assert_eq!(preflight.state, settings::FirstRunModelState::Blocked);
        assert!(!preflight.available);
        assert_eq!(preflight.destination_path, None);
        assert_eq!(preflight.expected_sha256, None);
        assert_eq!(preflight.source_count, 0);
        assert!(preflight.sources.is_empty());
        assert!(preflight
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("usable sha256")));
        assert!(preflight.operator_action.contains("models/registry.json"));
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn runtime_snapshot_rechecks_models_after_artifact_install() {
        let app_data = tmp();
        let registry_path =
            write_first_run_registry(&app_data, &sha256_for(b"asr"), &sha256_for(b"vad"));
        let state = RuntimeSnapshot::default();

        let initial = state.refresh_models(&registry_path, &app_data).unwrap();

        assert!(!initial.settings.first_run.model_ready);
        assert!(initial
            .settings
            .first_run
            .required_models
            .iter()
            .all(|model| model.state == settings::FirstRunModelState::Missing));

        let models_dir = app_data.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("fixture-asr.onnx"), b"asr").unwrap();

        let refreshed = state.refresh_models(&registry_path, &app_data).unwrap();

        assert!(refreshed.settings.first_run.model_ready);
        assert_eq!(refreshed.settings.first_run.model_readiness_error, None);
        assert!(refreshed
            .settings
            .first_run
            .required_models
            .iter()
            .all(|model| model.state == settings::FirstRunModelState::Ready));
        assert!(state.snapshot().settings.first_run.model_ready);
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn runtime_snapshot_installs_reviewed_model_artifacts() {
        let app_data = tmp();
        let source_dir = tmp();
        let registry_path =
            write_first_run_registry(&app_data, &sha256_for(b"asr"), &sha256_for(b"vad"));
        let asr_source = source_dir.join("reviewed-asr.onnx");
        std::fs::write(&asr_source, b"asr").unwrap();
        let state = RuntimeSnapshot::default();

        let asr_snapshot = state
            .install_model_artifact(&registry_path, &app_data, "fixture-asr", &asr_source)
            .unwrap();

        assert!(asr_snapshot.settings.first_run.model_ready);
        assert_eq!(
            std::fs::read(app_data.join("models/fixture-asr.onnx")).unwrap(),
            b"asr"
        );
        assert_eq!(
            asr_snapshot
                .settings
                .first_run
                .required_models
                .iter()
                .find(|model| model.id == "fixture-asr")
                .unwrap()
                .state,
            settings::FirstRunModelState::Ready
        );

        assert!(asr_snapshot
            .settings
            .first_run
            .required_models
            .iter()
            .all(|model| model.state == settings::FirstRunModelState::Ready));
        let _ = std::fs::remove_dir_all(app_data);
        let _ = std::fs::remove_dir_all(source_dir);
    }

    #[test]
    fn runtime_snapshot_refuses_unreviewed_model_artifact() {
        let app_data = tmp();
        let source_dir = tmp();
        let registry_path =
            write_first_run_registry(&app_data, &sha256_for(b"asr"), &sha256_for(b"vad"));
        let source = source_dir.join("wrong-asr.onnx");
        std::fs::write(&source, b"wrong").unwrap();
        let state = RuntimeSnapshot::default();

        let err = state
            .install_model_artifact(&registry_path, &app_data, "fixture-asr", &source)
            .unwrap_err();

        assert!(matches!(
            err,
            InstallModelArtifactError::ModelRegistry(
                models::ModelRegistryError::InstallChecksumMismatch { .. }
            )
        ));
        assert!(!app_data.join("models/fixture-asr.onnx").exists());
        assert!(!state.snapshot().settings.first_run.model_ready);
        let _ = std::fs::remove_dir_all(app_data);
        let _ = std::fs::remove_dir_all(source_dir);
    }

    #[test]
    fn runtime_snapshot_blocks_missing_model_without_download_sources() {
        let app_data = tmp();
        let registry_dir = app_data.join("registry");
        std::fs::create_dir_all(&registry_dir).unwrap();
        let registry_path = registry_dir.join("registry.json");
        std::fs::write(
            &registry_path,
            format!(
                r#"{{
                  "schema_version": 1,
                  "models": [
                    {{
                      "id": "fixture-asr",
                      "task": "asr",
                      "lane": "cpu",
                      "runtime": "onnxruntime",
                      "file": "fixture-asr.onnx",
                      "sha256": "{}",
                      "size_mb": 1,
                      "license": "Apache-2.0",
                      "min_hw": "any",
                      "recommended": true,
                      "sources": ["TODO_primary"]
                    }}
                  ]
                }}"#,
                sha256_for(b"asr")
            ),
        )
        .unwrap();
        let state = RuntimeSnapshot::default();

        state.refresh_model_readiness(&registry_path, &app_data.join("models"));

        let first_run = state.snapshot().settings.first_run;
        assert!(!first_run.model_ready);
        assert_eq!(
            first_run.model_readiness_error,
            Some(
                "Model registry needs verified metadata, checksums, or artifact repair".to_string()
            )
        );
        assert_eq!(first_run.required_models.len(), 1);
        assert_eq!(
            first_run.required_models[0].state,
            settings::FirstRunModelState::Blocked
        );
        assert!(first_run.required_models[0]
            .detail
            .contains("Download unavailable"));
        assert!(!first_run.required_models[0].download_available);
        assert_eq!(first_run.required_models[0].download_size_mb, None);
        assert_eq!(first_run.required_models[0].download_source_count, 0);
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn runtime_snapshot_marks_verified_first_run_models_ready() {
        let app_data = tmp();
        let models_dir = app_data.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("fixture-asr.onnx"), b"asr").unwrap();
        let registry_path =
            write_first_run_registry(&app_data, &sha256_for(b"asr"), &sha256_for(b"vad"));
        let state = RuntimeSnapshot::default();

        state.refresh_model_readiness(&registry_path, &models_dir);

        let first_run = state.snapshot().settings.first_run;
        assert!(first_run.model_ready);
        assert_eq!(first_run.model_readiness_error, None);
        assert_eq!(first_run.required_models.len(), 1);
        assert!(first_run
            .required_models
            .iter()
            .all(|model| model.state == settings::FirstRunModelState::Ready));
        assert!(first_run.model_ready);
        assert!(first_run.asr_candidates[0].selected);
        assert_eq!(
            first_run.asr_runtime.state,
            settings::FirstRunAsrRuntimeState::VerifiedArtifact
        );
        assert_eq!(first_run.asr_runtime.artifact_size_bytes, Some(3));
        assert!(!first_run.asr_runtime.adapter_ready);
        assert!(first_run
            .asr_runtime
            .proof_requirement
            .contains("Do not claim first-dictation readiness"));

        state.mark_asr_adapter_ready(engine::EngineLane::LocalCpu);
        let warmed = state.snapshot().settings.first_run.asr_runtime;
        assert!(warmed.adapter_ready);
        assert_eq!(warmed.lane.as_deref(), Some("cpu"));
        assert!(warmed.detail.contains("loaded and warmed"));
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn runtime_snapshot_quarantines_mismatched_first_run_model() {
        let app_data = tmp();
        let models_dir = app_data.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let mismatched_path = models_dir.join("fixture-asr.onnx");
        std::fs::write(&mismatched_path, b"unexpected").unwrap();
        std::fs::write(models_dir.join("fixture-vad.onnx"), b"vad").unwrap();
        let registry_path =
            write_first_run_registry(&app_data, &sha256_for(b"expected"), &sha256_for(b"vad"));
        let state = RuntimeSnapshot::default();

        state.refresh_model_readiness(&registry_path, &models_dir);

        let first_run = state.snapshot().settings.first_run;
        let asr_status = first_run
            .required_models
            .iter()
            .find(|model| model.id == "fixture-asr")
            .unwrap();
        assert_eq!(asr_status.state, settings::FirstRunModelState::Blocked);
        assert!(asr_status.detail.contains("quarantined at"));
        assert!(!mismatched_path.exists());
        assert!(models_dir.join(models::QUARANTINE_DIR_NAME).exists());
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn os_default_asr_recommendation_beats_cpu_fallback() {
        let registry = models::ModelRegistry::from_json(
            r#"{
              "schema_version": 1,
              "models": [
                {
                  "id": "cpu-safe",
                  "task": "asr",
                  "lane": "cpu",
                  "runtime": "onnxruntime",
                  "file": "cpu.onnx",
                  "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                  "size_mb": 1,
                  "license": "Apache-2.0",
                  "min_hw": "any",
                  "recommended": true,
                  "sources": ["https:\u002f\u002fmodels.example.test/cpu.onnx"]
                },
                {
                  "id": "mac-gpu",
                  "task": "asr",
                  "lane": "gpu",
                  "runtime": "whisper.cpp",
                  "file": "gpu.bin",
                  "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
                  "size_mb": 1,
                  "license": "MIT",
                  "min_hw": "metal_or_dgpu",
                  "recommended": true,
                  "default_for": ["macos"],
                  "sources": ["https:\u002f\u002fmodels.example.test/gpu.bin"]
                }
              ]
            }"#,
        )
        .unwrap();

        let recommended = first_run_asr_recommendation(&registry, "macos").unwrap();

        assert_eq!(recommended.id, "mac-gpu");
        assert_eq!(
            recommendation_reason(recommended, "macos"),
            "Recommended for this OS lane"
        );
    }

    #[test]
    fn current_registry_promotes_the_footprint_safe_model_only_on_proven_platforms() {
        let registry = models::ModelRegistry::load(&models::source_tree_registry_path()).unwrap();

        for platform in ["macos", "windows", "linux"] {
            let recommended = first_run_asr_recommendation(&registry, platform).unwrap();
            assert_eq!(recommended.id, "whisper-base-en-q5_1");
            assert_eq!(
                recommendation_reason(recommended, platform),
                "Recommended for this OS lane"
            );
        }
    }

    #[test]
    fn selecting_asr_model_updates_snapshot_and_persists_settings() {
        let app_data = tmp();
        let registry_path = write_selectable_asr_registry(&app_data);
        let state = RuntimeSnapshot::default();
        state.set_settings_store(&app_data);
        state.refresh_model_readiness(&registry_path, &app_data.join("models"));

        let snapshot = state.select_asr_model("fixture-gpu").unwrap();

        let first_run = snapshot.settings.first_run;
        assert_eq!(
            first_run.selected_asr_model_id.as_deref(),
            Some("fixture-gpu")
        );
        assert!(first_run
            .asr_candidates
            .iter()
            .any(|candidate| candidate.id == "fixture-gpu" && candidate.selected));
        assert_eq!(
            first_run.asr_runtime.selected_model_id.as_deref(),
            Some("fixture-gpu")
        );
        assert_eq!(
            first_run.asr_runtime.state,
            settings::FirstRunAsrRuntimeState::Pending
        );
        let expected_lane = if cfg!(all(target_os = "macos", feature = "asr-whisper-metal")) {
            "gpu"
        } else {
            "cpu"
        };
        assert_eq!(first_run.asr_runtime.lane.as_deref(), Some(expected_lane));
        assert!(first_run.asr_candidates.iter().any(|candidate| {
            candidate.id == "fixture-gpu" && candidate.lane.as_deref() == Some(expected_lane)
        }));
        let preflight = state
            .model_download_preflight(&registry_path, &app_data, "fixture-gpu")
            .unwrap();
        assert_eq!(preflight.lane.as_deref(), Some(expected_lane));
        assert!(first_run
            .asr_candidates
            .iter()
            .any(|candidate| candidate.id == "fixture-asr" && !candidate.selected));
        assert_eq!(
            settings::SettingsStore::new(&app_data)
                .load()
                .unwrap()
                .selected_asr_model_id
                .as_deref(),
            Some("fixture-gpu")
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn persisted_asr_selection_applies_after_model_refresh() {
        let app_data = tmp();
        let registry_path = write_ready_selectable_asr_registry(&app_data);
        let store = settings::SettingsStore::new(&app_data);
        store
            .save(&settings::UserSettingsFile {
                selected_asr_model_id: Some("fixture-gpu".to_string()),
                ..settings::UserSettingsFile::default()
            })
            .unwrap();
        let models_dir = app_data.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("fixture-gpu.bin"), b"gpu").unwrap();
        let state = RuntimeSnapshot::default();

        let snapshot = state.refresh_models(&registry_path, &app_data).unwrap();

        let first_run = snapshot.settings.first_run;
        assert_eq!(
            first_run.selected_asr_model_id.as_deref(),
            Some("fixture-gpu")
        );
        assert!(first_run
            .asr_candidates
            .iter()
            .any(|candidate| candidate.id == "fixture-gpu" && candidate.selected));
        assert!(first_run.model_ready);
        assert_eq!(first_run.required_models.len(), 1);
        assert_eq!(first_run.required_models[0].id, "fixture-gpu");
        assert_eq!(
            first_run.required_models[0].state,
            settings::FirstRunModelState::Ready
        );
        let expected_lane = if cfg!(all(target_os = "macos", feature = "asr-whisper-metal")) {
            "gpu"
        } else {
            "cpu"
        };
        assert_eq!(
            first_run.asr_runtime.state,
            settings::FirstRunAsrRuntimeState::VerifiedArtifact
        );
        assert_eq!(first_run.asr_runtime.lane.as_deref(), Some(expected_lane));
        assert_eq!(
            first_run.required_models[0].lane.as_deref(),
            Some(expected_lane)
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn persisted_hotkey_mode_applies_after_settings_load() {
        let app_data = tmp();
        let store = settings::SettingsStore::new(&app_data);
        store
            .save(&settings::UserSettingsFile {
                hotkey_mode: Some(settings::HotkeyModeSetting::Toggle),
                ..settings::UserSettingsFile::default()
            })
            .unwrap();
        let state = RuntimeSnapshot::default();
        state.set_settings_store(&app_data);

        state.apply_persisted_user_settings().unwrap();

        assert_eq!(
            state.snapshot().settings.hotkey.mode,
            settings::HotkeyModeSetting::Toggle
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn persisted_hotkey_binding_applies_after_settings_load() {
        let app_data = tmp();
        let store = settings::SettingsStore::new(&app_data);
        store
            .save(&settings::UserSettingsFile {
                hotkey_primary_binding: Some("Ctrl + Space".to_string()),
                ..settings::UserSettingsFile::default()
            })
            .unwrap();
        let state = RuntimeSnapshot::default();
        state.set_settings_store(&app_data);

        state.apply_persisted_user_settings().unwrap();

        assert_eq!(
            state.snapshot().settings.hotkey.primary_binding,
            "Control+Space"
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn persisted_cleanup_dial_applies_after_settings_load() {
        let app_data = tmp();
        let store = settings::SettingsStore::new(&app_data);
        store
            .save(&settings::UserSettingsFile {
                cleanup_default_dial: Some(CleanupDial::Full),
                ..settings::UserSettingsFile::default()
            })
            .unwrap();
        let state = RuntimeSnapshot::default();
        state.set_settings_store(&app_data);

        state.apply_persisted_user_settings().unwrap();

        assert_eq!(
            state.snapshot().settings.cleanup.default_dial,
            CleanupDial::Full
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_binding_persistence_normalizes_recommended_values() {
        let app_data = tmp();
        let store = settings::SettingsStore::new(&app_data);

        RuntimeSnapshot::persist_hotkey_binding(&store, "right-option").unwrap();

        assert_eq!(
            store.load().unwrap().hotkey_primary_binding.as_deref(),
            Some(settings::DEFAULT_HOTKEY_BINDING)
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn shortcut_mapping_accepts_recommended_bindings_only() {
        assert_eq!(
            shortcut_from_binding("RightAlt").unwrap(),
            shortcut_from_binding("right-option").unwrap()
        );
        assert_eq!(
            shortcut_from_binding("Ctrl + Space").unwrap(),
            shortcut_from_binding("Control+Space").unwrap()
        );
        assert!(shortcut_from_binding("F13").is_ok());
        assert!(shortcut_from_binding("F14").is_ok());
        assert!(shortcut_from_binding("Shift+F13").is_ok());
        assert!(matches!(
            shortcut_from_binding("CapsLock"),
            Err(settings::SettingsError::InvalidHotkeyBinding(binding))
                if binding == "CapsLock"
        ));
    }

    #[test]
    fn cleanup_override_shortcut_mapping_accepts_shift_right_alt_only() {
        assert_eq!(
            cleanup_override_shortcut_from_binding("Shift + RightAlt").unwrap(),
            cleanup_override_shortcut_from_binding("shift-right-option").unwrap()
        );
        assert!(matches!(
            cleanup_override_shortcut_from_binding("F13"),
            Err(settings::SettingsError::InvalidHotkeyBinding(binding)) if binding == "F13"
        ));
    }

    #[test]
    fn selecting_hotkey_mode_updates_snapshot_and_persists_settings() {
        let app_data = tmp();
        let state = RuntimeSnapshot::default();
        let runtime = HotkeyRuntimeHandle::default();

        let snapshot = state
            .set_hotkey_mode(settings::HotkeyModeSetting::Toggle, &app_data, &runtime)
            .unwrap();

        assert_eq!(
            snapshot.settings.hotkey.mode,
            settings::HotkeyModeSetting::Toggle
        );
        assert_eq!(
            settings::SettingsStore::new(&app_data)
                .load()
                .unwrap()
                .hotkey_mode,
            Some(settings::HotkeyModeSetting::Toggle)
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn selecting_cleanup_dial_updates_snapshot_and_persists_settings() {
        let app_data = tmp();
        let state = RuntimeSnapshot::default();
        let runtime = HotkeyRuntimeHandle::default();

        let snapshot = state
            .set_cleanup_dial(CleanupDial::Raw, &app_data, &runtime)
            .unwrap();

        assert_eq!(snapshot.settings.cleanup.default_dial, CleanupDial::Raw);
        assert_eq!(
            settings::SettingsStore::new(&app_data)
                .load()
                .unwrap()
                .cleanup_default_dial,
            Some(CleanupDial::Raw)
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_handle_refuses_binding_change_during_capture() {
        let app_data = tmp();
        let runtime = Arc::new(Mutex::new(HotkeyRuntime::new_wal_only(&app_data).unwrap()));
        let handle = HotkeyRuntimeHandle::default();
        handle.set_runtime(Arc::clone(&runtime));

        runtime
            .lock()
            .unwrap()
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();

        let err = handle.ensure_idle().unwrap_err();

        assert!(matches!(err, HotkeyBindingUpdateError::CaptureActive));
        let _ = runtime
            .lock()
            .unwrap()
            .handle_signal(hotkeys::Signal::Release { at_ms: 400 });
        let _ = runtime
            .lock()
            .unwrap()
            .handle_signal(hotkeys::Signal::Tick { at_ms: 700 });
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn right_alt_role_follows_the_bound_shortcuts() {
        let handle = HotkeyRuntimeHandle::default();
        assert_eq!(handle.right_alt_role(false), None);

        handle.set_primary_shortcut(shortcut_from_binding("RightAlt").unwrap());
        handle.set_cleanup_override_shortcut(Some(
            cleanup_override_shortcut_from_binding("Shift + RightAlt").unwrap(),
        ));
        assert_eq!(
            handle.right_alt_role(false),
            Some(HotkeyShortcutRole::Primary)
        );
        assert_eq!(
            handle.right_alt_role(true),
            Some(HotkeyShortcutRole::CleanupOverride)
        );

        // Rebound away from Right-Alt: a bare press is no longer ours.
        handle.set_primary_shortcut(shortcut_from_binding("F13").unwrap());
        assert_eq!(handle.right_alt_role(false), None);
        assert_eq!(
            handle.right_alt_role(true),
            Some(HotkeyShortcutRole::CleanupOverride)
        );
    }

    #[test]
    fn only_windows_serves_right_alt_natively() {
        let right_alt = shortcut_from_binding("RightAlt").unwrap();
        let shift_right_alt = cleanup_override_shortcut_from_binding("Shift + RightAlt").unwrap();
        let windows = cfg!(target_os = "windows");
        assert_eq!(served_by_native_listener(&right_alt), windows);
        assert_eq!(served_by_native_listener(&shift_right_alt), windows);
        for binding in ["F13", "F14", "Control+Space", "Shift+F13"] {
            assert!(!served_by_native_listener(
                &shortcut_from_binding(binding).unwrap()
            ));
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn right_alt_registration_reports_the_listener_outcome() {
        let handle = HotkeyRuntimeHandle::default();
        assert!(handle.native_listener_status().is_err()); // not started yet
        *handle.native_listener.lock().unwrap() = Some(Err("no raw input".to_string()));
        assert_eq!(
            handle.native_listener_status(),
            Err("no raw input".to_string())
        );
        *handle.native_listener.lock().unwrap() = Some(Ok(()));
        assert_eq!(handle.native_listener_status(), Ok(()));
    }

    #[test]
    fn selecting_unknown_asr_model_is_rejected() {
        let app_data = tmp();
        let registry_path = write_selectable_asr_registry(&app_data);
        let state = RuntimeSnapshot::default();
        state.set_settings_store(&app_data);
        state.refresh_model_readiness(&registry_path, &app_data.join("models"));

        let err = state.select_asr_model("missing-model").unwrap_err();

        assert!(matches!(err, SelectModelError::UnknownModel(model) if model == "missing-model"));
        assert_eq!(
            settings::SettingsStore::new(&app_data)
                .load()
                .unwrap()
                .selected_asr_model_id,
            None
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn selected_asr_runtime_state_blocks_missing_artifact() {
        let app_data = tmp();
        let registry_path =
            write_first_run_registry(&app_data, &sha256_for(b"asr"), &sha256_for(b"vad"));
        let models_dir = app_data.join("models");
        let state = RuntimeSnapshot::default();
        state.refresh_model_readiness(&registry_path, &models_dir);
        let first_run = state.snapshot().settings.first_run;

        let runtime_state = selected_asr_runtime_state(&first_run, &registry_path, &models_dir);

        assert!(matches!(
            runtime_state,
            engine::LocalAsrAdapterState::Blocked {
                selected_model_id: Some(ref model_id),
                lane: engine::EngineLane::LocalCpu,
                ref reason,
            } if model_id == "fixture-asr"
                && reason.contains("selected artifact is missing at")
                && reason.contains("fixture-asr.onnx")
        ));
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn selected_asr_runtime_state_detects_verified_artifact() {
        let app_data = tmp();
        let models_dir = app_data.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let artifact_path = models_dir.join("fixture-asr.onnx");
        std::fs::write(&artifact_path, b"asr").unwrap();
        std::fs::write(models_dir.join("fixture-vad.onnx"), b"vad").unwrap();
        let registry_path =
            write_first_run_registry(&app_data, &sha256_for(b"asr"), &sha256_for(b"vad"));
        let state = RuntimeSnapshot::default();
        state.refresh_model_readiness(&registry_path, &models_dir);
        let first_run = state.snapshot().settings.first_run;

        let runtime_state = selected_asr_runtime_state(&first_run, &registry_path, &models_dir);

        assert_eq!(
            runtime_state,
            engine::LocalAsrAdapterState::VerifiedArtifact {
                spec: engine::LocalAsrAdapterSpec {
                    model_id: "fixture-asr".to_string(),
                    lane: engine::EngineLane::LocalCpu,
                    runtime: "onnxruntime".to_string(),
                    artifact_path,
                    artifact_size_bytes: 3,
                }
            }
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_refuses_asr_adapter_swap_during_capture() {
        let app_data = tmp();
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data).unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();

        let err = runtime
            .set_asr_adapter_state(engine::LocalAsrAdapterState::Pending {
                selected_model_id: Some("fixture-asr".to_string()),
                lane: engine::EngineLane::LocalCpu,
            })
            .unwrap_err();

        assert!(matches!(err, AsrRuntimeUpdateError::CaptureActive));
        let _ = runtime.handle_signal(hotkeys::Signal::Release { at_ms: 400 });
        let _ = runtime.handle_signal(hotkeys::Signal::Tick { at_ms: 700 });
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_warms_the_capture_processor_before_dictation() {
        let app_data = tmp();
        let calls = Arc::new(Mutex::new(0));
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data)
            .unwrap()
            .with_processor(Box::new(WarmupProcessor {
                calls: Arc::clone(&calls),
            }));

        let lane = runtime.warm_up_asr().unwrap();

        assert_eq!(lane, Some(engine::EngineLane::LocalGpu));
        assert_eq!(*calls.lock().unwrap(), 1);
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn asr_warmup_failure_returns_the_blocked_snapshot_to_setup() {
        let state = RuntimeSnapshot::default();
        let warmup_error = AsrRuntimeUpdateError::Warmup(pipeline::PipelineError::Asr(
            engine::AsrError::Unavailable("Metal initialization failed".to_string()),
        ));

        let snapshot = complete_asr_runtime_update(&state, Err(warmup_error)).unwrap();

        assert_eq!(
            snapshot.settings.first_run.asr_runtime.state,
            settings::FirstRunAsrRuntimeState::Blocked
        );
        assert!(!snapshot.settings.first_run.asr_runtime.adapter_ready);
        assert!(snapshot
            .settings
            .first_run
            .asr_runtime
            .detail
            .contains("Metal initialization failed"));

        let lock_error =
            complete_asr_runtime_update(&state, Err(AsrRuntimeUpdateError::RuntimePoisoned))
                .unwrap_err();
        assert_eq!(lock_error, "ASR runtime lock poisoned");
    }

    #[test]
    fn hotkey_runtime_switches_to_toggle_mode_when_idle() {
        let app_data = tmp();
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data).unwrap();

        runtime
            .set_hotkey_mode(
                settings::HotkeyModeSetting::Toggle,
                settings::CaptureSettings::default(),
            )
            .unwrap();

        assert_eq!(
            runtime
                .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
                .unwrap(),
            None
        );
        assert!(matches!(
            runtime.coordinator.state(),
            hotkeys::CaptureState::Capturing { .. }
        ));
        assert_eq!(
            runtime
                .handle_signal(hotkeys::Signal::Release { at_ms: 200 })
                .unwrap(),
            None
        );
        assert!(matches!(
            runtime.coordinator.state(),
            hotkeys::CaptureState::Capturing { .. }
        ));
        assert_eq!(
            runtime
                .handle_signal(hotkeys::Signal::Press { at_ms: 500 })
                .unwrap(),
            Some(800)
        );
        assert!(matches!(
            runtime.coordinator.state(),
            hotkeys::CaptureState::Finalizing { ends_ms: 800, .. }
        ));
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_refuses_mode_change_during_capture() {
        let app_data = tmp();
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data).unwrap();

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();
        let err = runtime
            .set_hotkey_mode(
                settings::HotkeyModeSetting::Toggle,
                settings::CaptureSettings::default(),
            )
            .unwrap_err();

        assert!(matches!(err, HotkeyModeUpdateError::CaptureActive));
        let _ = runtime.handle_signal(hotkeys::Signal::Release { at_ms: 400 });
        let _ = runtime.handle_signal(hotkeys::Signal::Tick { at_ms: 700 });
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_uses_cleanup_default_for_default_profile() {
        let app_data = tmp();
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let seen_dials = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data)
            .unwrap()
            .with_target_resolver(Box::new(profiles::SessionTargetResolver::new(
                profiles::StaticFrontmostAppDetector::new(detected_app()),
                profiles::ProfileStore::default(),
            )))
            .with_processor(Box::new(ScriptedProcessor::new(Arc::clone(&seen_dials))))
            .with_injector(Box::new(TestInjector::native(Arc::clone(&delivered))));
        runtime.set_default_cleanup_dial(CleanupDial::Full).unwrap();

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();
        let id = runtime.recorder.active_session_id().unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Release { at_ms: 400 })
            .unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Tick { at_ms: 700 })
            .unwrap();

        assert_eq!(*seen_dials.lock().unwrap(), vec![CleanupDial::Full]);
        let session = runtime.history.get_session(id).unwrap().unwrap();
        assert_eq!(session.clean_text.as_deref(), Some("Hello captain."));
        assert_eq!(session.cleanup_dial, Some(CleanupDial::Full));
        assert_eq!(*delivered.lock().unwrap(), vec!["Hello captain."]);
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_keeps_user_edited_profile_cleanup_over_default() {
        let app_data = tmp();
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let seen_dials = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data)
            .unwrap()
            .with_target_resolver(Box::new(profiles::SessionTargetResolver::new(
                profiles::StaticFrontmostAppDetector::new(detected_app()),
                matching_profiles(CleanupDial::Full),
            )))
            .with_processor(Box::new(ScriptedProcessor::new(Arc::clone(&seen_dials))))
            .with_injector(Box::new(TestInjector::native(Arc::clone(&delivered))));
        runtime.set_default_cleanup_dial(CleanupDial::Raw).unwrap();

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();
        let id = runtime.recorder.active_session_id().unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Release { at_ms: 400 })
            .unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Tick { at_ms: 700 })
            .unwrap();

        assert_eq!(*seen_dials.lock().unwrap(), vec![CleanupDial::Full]);
        let session = runtime.history.get_session(id).unwrap().unwrap();
        assert_eq!(session.cleanup_dial, Some(CleanupDial::Full));
        assert_eq!(*delivered.lock().unwrap(), vec!["Hello captain."]);
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_refuses_cleanup_default_change_during_capture() {
        let app_data = tmp();
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data).unwrap();

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();
        let err = runtime
            .set_default_cleanup_dial(CleanupDial::Raw)
            .unwrap_err();

        assert!(matches!(err, CleanupDialUpdateError::CaptureActive));
        let _ = runtime.handle_signal(hotkeys::Signal::Release { at_ms: 400 });
        let _ = runtime.handle_signal(hotkeys::Signal::Tick { at_ms: 700 });
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn recover_history_audio_surfaces_untracked_wal_once() {
        let app_data = tmp();
        let id = events::SessionId::new(ulid::Ulid::new());
        let sessions_dir = app_data.join("sessions");
        let mut writer = audio::wal::WalWriter::create(&sessions_dir, &id.0.to_string()).unwrap();
        writer.append(&vec![0.1f32; 240]).unwrap();
        let wal_path = writer.path().to_path_buf();
        drop(writer);
        let mut store = history::HistoryStore::open(&app_data).unwrap();

        assert_eq!(recover_history_audio(&mut store, &app_data).unwrap(), 1);
        assert_eq!(recover_history_audio(&mut store, &app_data).unwrap(), 0);

        let session = store.get_session(id).unwrap().unwrap();
        assert_eq!(
            session.audio_path.as_deref(),
            Some(wal_path.to_str().unwrap())
        );
        assert_eq!(
            session.failure.as_ref().map(|failure| failure.stage),
            Some(events::Stage::Capture)
        );
        assert!(session
            .failure
            .as_ref()
            .unwrap()
            .error
            .contains("240 samples preserved"));
        assert_eq!(session.event_count, 2);
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_processes_finalized_capture_and_holds_when_no_target() {
        let app_data = tmp();
        let seen_dials = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data)
            .unwrap()
            .with_target_resolver(Box::new(profiles::SessionTargetResolver::new(
                profiles::StaticFrontmostAppDetector::new(detected_app()),
                matching_profiles(CleanupDial::Full),
            )))
            .with_processor(Box::new(ScriptedProcessor::new(Arc::clone(&seen_dials))))
            .with_injector(Box::new(TestInjector::no_target()));

        assert_eq!(
            runtime
                .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
                .unwrap(),
            None
        );
        let id = runtime.recorder.active_session_id().unwrap();
        assert_eq!(
            runtime
                .handle_signal(hotkeys::Signal::Release { at_ms: 400 })
                .unwrap(),
            Some(700)
        );
        assert_eq!(
            runtime
                .handle_signal(hotkeys::Signal::Tick { at_ms: 700 })
                .unwrap(),
            None
        );

        let path = app_data.join("sessions").join(format!("{}.wav", id.0));
        assert!(runtime.recorder.active_session_id().is_none());
        assert!(path.exists());
        assert_eq!(&std::fs::read(&path).unwrap()[0..4], b"RIFF");
        assert!(app_data.join(history::HISTORY_DB_FILE).exists());

        let session = runtime.history.get_session(id).unwrap().unwrap();
        assert_eq!(session.target_app, Some(detected_app()));
        assert_eq!(session.audio_path.as_deref(), Some(path.to_str().unwrap()));
        assert_eq!(session.raw_text.as_deref(), Some("um hello captain"));
        assert_eq!(session.clean_text.as_deref(), Some("Hello captain."));
        assert_eq!(session.cleanup_dial, Some(CleanupDial::Full));
        assert_eq!(session.held_reason, Some(HoldReason::NoTarget));
        assert_eq!(session.event_count, 5);
        assert_eq!(*seen_dials.lock().unwrap(), vec![CleanupDial::Full]);
        assert_eq!(
            runtime.history.events_for_session(id).unwrap(),
            vec![
                events::SessionEvent::Started {
                    id,
                    target_app: detected_app(),
                    at_ms: 0,
                },
                audio::CaptureSessionSummary {
                    id,
                    wal_path: path,
                    samples_written: 0,
                    dropped_input_samples: 0,
                    started_ms: 0,
                    finalized_ms: 700
                }
                .audio_persisted_event(),
                events::SessionEvent::RawFinal {
                    id,
                    text: "um hello captain".to_string(),
                },
                events::SessionEvent::CleanFinal {
                    id,
                    text: "Hello captain.".to_string(),
                    dial: CleanupDial::Full,
                },
                events::SessionEvent::Held {
                    id,
                    reason: HoldReason::NoTarget,
                },
            ]
        );
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_injects_committed_text_when_backend_succeeds() {
        let app_data = tmp();
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let seen_dials = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data)
            .unwrap()
            .with_target_resolver(Box::new(profiles::SessionTargetResolver::new(
                profiles::StaticFrontmostAppDetector::new(detected_app()),
                matching_profiles(CleanupDial::Light),
            )))
            .with_processor(Box::new(ScriptedProcessor::new(Arc::clone(&seen_dials))))
            .with_injector(Box::new(TestInjector::native(Arc::clone(&delivered))));

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();
        let id = runtime.recorder.active_session_id().unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Release { at_ms: 400 })
            .unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Tick { at_ms: 700 })
            .unwrap();

        assert_eq!(*delivered.lock().unwrap(), vec!["Hello captain."]);
        let session = runtime.history.get_session(id).unwrap().unwrap();
        assert_eq!(session.injected_method, Some(InjectMethod::Native));
        assert_eq!(
            runtime.history.events_for_session(id).unwrap().last(),
            Some(&events::SessionEvent::Injected {
                id,
                method: InjectMethod::Native,
            })
        );
        assert!(runtime.take_first_dictation_completion());
        assert!(!runtime.take_first_dictation_completion());
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_cleanup_override_uses_raw_for_one_capture() {
        let app_data = tmp();
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let seen_dials = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data)
            .unwrap()
            .with_target_resolver(Box::new(profiles::SessionTargetResolver::new(
                profiles::StaticFrontmostAppDetector::new(detected_app()),
                matching_profiles(CleanupDial::Light),
            )))
            .with_processor(Box::new(ScriptedProcessor::new(Arc::clone(&seen_dials))))
            .with_injector(Box::new(TestInjector::native(Arc::clone(&delivered))));

        runtime
            .handle_shortcut_signal(
                HotkeyShortcutRole::CleanupOverride,
                hotkeys::Signal::Press { at_ms: 0 },
            )
            .unwrap();
        let raw_id = runtime.recorder.active_session_id().unwrap();
        assert_eq!(
            runtime
                .handle_signal(hotkeys::Signal::Release { at_ms: 400 })
                .unwrap(),
            None
        );
        assert!(matches!(
            runtime.coordinator.state(),
            hotkeys::CaptureState::Capturing { .. }
        ));
        assert_eq!(
            runtime
                .handle_shortcut_signal(
                    HotkeyShortcutRole::CleanupOverride,
                    hotkeys::Signal::Release { at_ms: 500 },
                )
                .unwrap(),
            Some(800)
        );
        runtime
            .handle_signal(hotkeys::Signal::Tick { at_ms: 800 })
            .unwrap();

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 1_000 })
            .unwrap();
        let light_id = runtime.recorder.active_session_id().unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Release { at_ms: 1_400 })
            .unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Tick { at_ms: 1_700 })
            .unwrap();

        assert_eq!(
            *delivered.lock().unwrap(),
            vec!["um hello captain", "Hello captain."]
        );
        assert_eq!(
            *seen_dials.lock().unwrap(),
            vec![CleanupDial::Raw, CleanupDial::Light]
        );

        let raw_session = runtime.history.get_session(raw_id).unwrap().unwrap();
        assert_eq!(raw_session.raw_text.as_deref(), Some("um hello captain"));
        assert_eq!(raw_session.clean_text, None);
        assert_eq!(raw_session.cleanup_dial, None);
        assert_eq!(raw_session.injected_method, Some(InjectMethod::Native));

        let light_session = runtime.history.get_session(light_id).unwrap().unwrap();
        assert_eq!(light_session.raw_text.as_deref(), Some("um hello captain"));
        assert_eq!(light_session.clean_text.as_deref(), Some("Hello captain."));
        assert_eq!(light_session.cleanup_dial, Some(CleanupDial::Light));
        assert_eq!(light_session.injected_method, Some(InjectMethod::Native));
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_first_run_proof_keeps_zero_sample_microphone_pending() {
        let app_data = tmp();
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let seen_dials = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data)
            .unwrap()
            .with_target_resolver(Box::new(profiles::SessionTargetResolver::new(
                profiles::StaticFrontmostAppDetector::new(detected_app()),
                matching_profiles(CleanupDial::Light),
            )))
            .with_processor(Box::new(ScriptedProcessor::new(Arc::clone(&seen_dials))))
            .with_injector(Box::new(TestInjector::native(Arc::clone(&delivered))));

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Release { at_ms: 400 })
            .unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Tick { at_ms: 700 })
            .unwrap();

        let proof = runtime.take_first_run_proof();

        assert!(proof.first_dictation_completed);
        assert!(!proof.microphone_permission_ready);
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_holds_when_focus_changes_before_delivery() {
        let app_data = tmp();
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let seen_dials = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data)
            .unwrap()
            .with_target_resolver(Box::new(QueueTargetResolver::new(vec![
                session_target(detected_app(), CleanupDial::Light),
                session_target(other_app(), CleanupDial::Light),
            ])))
            .with_processor(Box::new(ScriptedProcessor::new(Arc::clone(&seen_dials))))
            .with_injector(Box::new(TestInjector::native(Arc::clone(&delivered))));

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();
        let id = runtime.recorder.active_session_id().unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Release { at_ms: 400 })
            .unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Tick { at_ms: 700 })
            .unwrap();

        assert!(delivered.lock().unwrap().is_empty());
        let session = runtime.history.get_session(id).unwrap().unwrap();
        assert_eq!(session.held_reason, Some(HoldReason::FocusChanged));
        assert_eq!(session.injected_method, None);
        assert_eq!(
            runtime.history.events_for_session(id).unwrap().last(),
            Some(&events::SessionEvent::Held {
                id,
                reason: HoldReason::FocusChanged,
            })
        );
        assert!(!runtime.take_first_dictation_completion());
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_holds_when_target_identity_is_unknown() {
        let app_data = tmp();
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let seen_dials = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data)
            .unwrap()
            .with_target_resolver(Box::new(profiles::SessionTargetResolver::new(
                profiles::StaticFrontmostAppDetector::unknown(),
                profiles::ProfileStore::default(),
            )))
            .with_processor(Box::new(ScriptedProcessor::new(Arc::clone(&seen_dials))))
            .with_injector(Box::new(TestInjector::native(Arc::clone(&delivered))));

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();
        let id = runtime.recorder.active_session_id().unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Release { at_ms: 400 })
            .unwrap();
        runtime
            .handle_signal(hotkeys::Signal::Tick { at_ms: 700 })
            .unwrap();

        assert!(delivered.lock().unwrap().is_empty());
        let session = runtime.history.get_session(id).unwrap().unwrap();
        assert_eq!(session.target_app, Some(profiles::unknown_app_ref()));
        assert_eq!(session.held_reason, Some(HoldReason::FocusChanged));
        assert_eq!(session.injected_method, None);
        let _ = std::fs::remove_dir_all(app_data);
    }

    #[test]
    fn hotkey_runtime_discards_short_tap_wal() {
        let app_data = tmp();
        let mut runtime = HotkeyRuntime::new_wal_only(&app_data).unwrap();

        runtime
            .handle_signal(hotkeys::Signal::Press { at_ms: 0 })
            .unwrap();
        let id = runtime.recorder.active_session_id().unwrap();
        let path = app_data.join("sessions").join(format!("{}.wav", id.0));
        assert!(path.exists());

        assert_eq!(
            runtime
                .handle_signal(hotkeys::Signal::Release { at_ms: 100 })
                .unwrap(),
            None
        );

        assert!(runtime.recorder.active_session_id().is_none());
        assert!(!path.exists());
        assert!(runtime.history.get_session(id).unwrap().is_none());
        assert!(runtime.history.events_for_session(id).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(app_data);
    }
}
