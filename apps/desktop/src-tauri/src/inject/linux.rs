//! Linux text-injection backend (P1-P0-3, ADR-0013, ADR-0023).
//!
//! ADR-0013 ladder, as it runs in the shipped app:
//!   1. **Detect** with AT-SPI2: a background focus tracker classifies the focused
//!      accessible (secure-field signal, non-negotiable #8). AT-SPI `InsertText`
//!      is not used for delivery — it no-ops on modern GNOME (ADR-0013 amendment).
//!   2. **Type** through the best keystroke channel this session offers:
//!      `zwp_virtual_keyboard_v1` (Hyprland/Omarchy, Sway, wlroots — rootless,
//!      full Unicode; `wayland_vk.rs`), else `/dev/uinput` when the user has
//!      opted into device access (GNOME/KDE; ASCII-only today; `uinput.rs`).
//!   3. **Clipboard** fallback is not offered on Linux yet (caps say so honestly).
//!
//! Focus truth: on Hyprland the compositor tells us the focused window and its
//! pid (`hyprland.rs`). An AT-SPI observation only counts when it comes from
//! that same process — a stale "Editable" from an app the user already left can
//! never vouch for the window that will actually receive the keys. Without
//! compositor focus (GNOME/KDE Wayland) only a *Secure* observation is honoured
//! (fail-safe); everything else is `Unknown` and follows the disclosed
//! Lenient/Strict policy.
#![cfg(target_os = "linux")]
#![allow(dead_code)]

use super::{FieldKind, InjectError, InjectorCaps, KeystrokeChannel, TextInjector};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Map an AT-SPI role + editable state to our platform-agnostic [`FieldKind`].
/// A password role is secure regardless of the editable bit; a non-editable
/// object is not an injection target. Pure + total.
pub fn role_to_field_kind(role: atspi::Role, editable: bool) -> FieldKind {
    match role {
        atspi::Role::PasswordText => FieldKind::Secure,
        _ if editable => FieldKind::Editable,
        _ => FieldKind::NoTarget,
    }
}

// ─────────────────────────────── focus classification ──────────────────────

/// The latest AT-SPI focus observation: what kind of field, in which process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusObservation {
    pub kind: FieldKind,
    pub pid: Option<u32>,
}

/// What the compositor reports about keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositorFocus {
    /// No compositor focus API (GNOME/KDE Wayland, or IPC unavailable).
    Unavailable,
    /// Compositor says no window holds focus (empty workspace, layer surface).
    NoWindow,
    /// A window holds focus; `pid` when the compositor knows it.
    Window { pid: Option<u32> },
}

/// Combine the AT-SPI observation with compositor focus into the field kind the
/// secure-field policy decides on. Pure and total — the whole trust model:
/// - compositor + a11y agree on the process → trust the a11y classification;
/// - they disagree / a11y silent → `Unknown` (opaque client);
/// - no compositor truth → honour only `Secure` (refusing is the safe error).
pub fn classify_focus(obs: Option<FocusObservation>, compositor: CompositorFocus) -> FieldKind {
    match compositor {
        CompositorFocus::Window {
            pid: Some(window_pid),
        } => match obs {
            Some(o) if o.pid == Some(window_pid) => o.kind,
            _ => FieldKind::Unknown,
        },
        CompositorFocus::Window { pid: None } | CompositorFocus::NoWindow => FieldKind::Unknown,
        CompositorFocus::Unavailable => match obs {
            Some(o) if o.kind == FieldKind::Secure => FieldKind::Secure,
            _ => FieldKind::Unknown,
        },
    }
}

// ─────────────────────────────── AT-SPI focus tracker ──────────────────────

/// The tracker's shared state: the latest focus observation, plus its AT-SPI
/// bus connection so delivery can run an on-demand lookup (see
/// [`lookup_focused`]).
struct FocusTracker {
    latest: Mutex<Option<TrackedFocus>>,
    conn: OnceLock<zbus::Connection>,
}

type FocusCell = Arc<FocusTracker>;

/// Bounds for the on-demand focused-element lookup. Delivery happens after
/// ASR, so a few tens of ms are affordable; a runaway tree (a browser) or a
/// hung app degrades to "no observation" → `Unknown`, never a stall.
const LOOKUP_MAX_NODES: usize = 3000;
const LOOKUP_BUDGET: Duration = Duration::from_millis(150);

/// Max cached bus-name → pid lookups before the cache is reset.
const PID_CACHE_LIMIT: usize = 512;

#[derive(Debug, Clone)]
struct TrackedFocus {
    bus_name: String,
    path: String,
    obs: FocusObservation,
}

/// One tracker per process: a background thread on the AT-SPI bus. Started only
/// inside a graphical session; if the bus is unavailable the thread exits and
/// every lookup is simply "no observation" (→ `Unknown`).
fn focus_tracker() -> Option<&'static FocusCell> {
    static TRACKER: OnceLock<Option<FocusCell>> = OnceLock::new();
    TRACKER
        .get_or_init(|| {
            let graphical = std::env::var_os("WAYLAND_DISPLAY").is_some()
                || std::env::var_os("DISPLAY").is_some();
            if !graphical {
                return None;
            }
            let cell: FocusCell = Arc::new(FocusTracker {
                latest: Mutex::new(None),
                conn: OnceLock::new(),
            });
            let publish = Arc::clone(&cell);
            std::thread::Builder::new()
                .name("atspi-focus".into())
                .spawn(move || {
                    if let Err(err) = pollster::block_on(track_focus(publish)) {
                        eprintln!("AT-SPI focus tracking unavailable: {err}");
                    }
                })
                .ok()?;
            Some(cell)
        })
        .as_ref()
}

async fn track_focus(publish: FocusCell) -> Result<(), Box<dyn std::error::Error>> {
    use atspi::connection::AccessibilityConnection;
    use atspi::events::object::StateChangedEvent;
    use atspi::proxy::accessible::AccessibleProxy;
    use atspi::State;
    use futures_lite::stream::StreamExt;
    use std::collections::HashMap;

    let conn = AccessibilityConnection::new().await?;
    conn.register_event::<StateChangedEvent>().await?;
    let _ = publish.conn.set(conn.connection().clone());
    let dbus = zbus::fdo::DBusProxy::new(conn.connection()).await?;
    let mut pids: HashMap<String, Option<u32>> = HashMap::new();
    let events = conn.event_stream();
    futures_lite::pin!(events);

    while let Some(ev) = events.next().await {
        let Ok(atspi::Event::Object(atspi::events::ObjectEvents::StateChanged(sc))) = ev else {
            continue;
        };
        if sc.state != State::Focused {
            continue;
        }
        let Some(name) = sc.item.name() else { continue };
        let bus_name = name.as_str().to_string();
        let path = sc.item.path().as_str().to_string();

        if !sc.enabled {
            // The tracked widget lost focus without a successor announcing
            // itself: forget it rather than let it vouch for the next field.
            if let Ok(mut slot) = publish.latest.lock() {
                if slot
                    .as_ref()
                    .is_some_and(|t| t.bus_name == bus_name && t.path == path)
                {
                    *slot = None;
                }
            }
            continue;
        }

        // A single app vanishing mid-event must never end tracking for the
        // whole session: every per-event failure just skips that event.
        let Ok(builder) = AccessibleProxy::builder(conn.connection())
            .destination(name.to_owned())
            .and_then(|b| b.path(sc.item.path().to_owned()))
        else {
            continue;
        };
        let Ok(acc) = builder.build().await else {
            continue;
        };
        let Ok(role) = acc.get_role().await else {
            continue;
        };
        let editable = matches!(acc.get_state().await, Ok(s) if s.contains(State::Editable));
        let pid = match pids.get(&bus_name) {
            Some(pid) => *pid,
            None => {
                let pid = dbus
                    .get_connection_unix_process_id(zbus::names::BusName::Unique(name.to_owned()))
                    .await
                    .ok();
                // Unique bus names are never reused, so entries only go stale;
                // bound the cache for very long sessions.
                if pids.len() >= PID_CACHE_LIMIT {
                    pids.clear();
                }
                pids.insert(bus_name.clone(), pid);
                pid
            }
        };
        let obs = FocusObservation {
            kind: role_to_field_kind(role, editable),
            pid,
        };
        if let Ok(mut slot) = publish.latest.lock() {
            *slot = Some(TrackedFocus {
                bus_name,
                path,
                obs,
            });
        }
    }
    Ok(())
}

// ─────────────────────────── on-demand focus lookup ────────────────────────
//
// Events alone miss a field that already had focus before the tracker
// connected (Kaydence launched while a password prompt was up) or whose focus
// event was dropped. Live-found 2026-09-26: that case classified a GTK4
// password entry as Unknown and Lenient typed into it. So when no event
// observation vouches for the compositor-focused process, delivery searches
// that process's accessibility tree for the element holding focus.

/// The process to search actively, if the event-driven observation cannot
/// vouch for the compositor-focused window. Pure.
pub fn lookup_pid(obs: Option<FocusObservation>, compositor: CompositorFocus) -> Option<u32> {
    match compositor {
        CompositorFocus::Window {
            pid: Some(window_pid),
        } if obs.and_then(|o| o.pid) != Some(window_pid) => Some(window_pid),
        _ => None,
    }
}

/// Find the focused accessible in `pid`'s tree, within [`LOOKUP_BUDGET`].
/// Runs on a helper thread so a hung application can never stall delivery;
/// any failure or timeout is "no observation".
fn lookup_focused(tracker: &FocusTracker, pid: u32) -> Option<FocusObservation> {
    let conn = tracker.conn.get()?.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("atspi-lookup".into())
        .spawn(move || {
            let deadline = Instant::now() + LOOKUP_BUDGET;
            let _ = tx.send(pollster::block_on(find_focused(&conn, pid, deadline)));
        })
        .ok()?;
    rx.recv_timeout(LOOKUP_BUDGET + Duration::from_millis(50))
        .ok()
        .flatten()
}

async fn find_focused(
    conn: &zbus::Connection,
    pid: u32,
    deadline: Instant,
) -> Option<FocusObservation> {
    use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
    use atspi::State;
    use std::collections::VecDeque;

    let registry = AccessibleProxy::builder(conn)
        .destination("org.a11y.atspi.Registry")
        .ok()?
        .path("/org/a11y/atspi/accessible/root")
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()?;
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    for app in registry.get_children().await.ok()? {
        if Instant::now() > deadline {
            return None;
        }
        let Some(name) = app.name() else { continue };
        let owner = zbus::names::BusName::Unique(name.clone());
        if dbus.get_connection_unix_process_id(owner).await.ok() != Some(pid) {
            continue;
        }
        let mut queue = VecDeque::from([(app, 0usize)]);
        let mut visited = 0usize;
        while let Some((node, depth)) = queue.pop_front() {
            if visited >= LOOKUP_MAX_NODES || Instant::now() > deadline {
                return None;
            }
            visited += 1;
            let Ok(acc) = node.as_accessible_proxy(conn).await else {
                continue;
            };
            let Ok(states) = acc.get_state().await else {
                continue;
            };
            if depth > 0 && states.contains(State::Focused) {
                let role = acc.get_role().await.ok()?;
                return Some(FocusObservation {
                    kind: role_to_field_kind(role, states.contains(State::Editable)),
                    pid: Some(pid),
                });
            }
            // Keyboard focus lives in a visible subtree; skip hidden branches.
            if depth > 0 && !states.contains(State::Showing) {
                continue;
            }
            if let Ok(children) = acc.get_children().await {
                queue.extend(children.into_iter().map(|child| (child, depth + 1)));
            }
        }
    }
    None
}

// ─────────────────────────────── the injector ──────────────────────────────

/// The shipped Linux injector. Channel detection runs once at construction.
pub struct LinuxTextInjector {
    keystroke: KeystrokeChannel,
    tracker: Option<&'static FocusCell>,
}

impl LinuxTextInjector {
    /// Detect this session's best keystroke channel and start focus tracking.
    pub fn detect() -> Self {
        let keystroke = if super::wayland_vk::VirtualKeyboard::available() {
            KeystrokeChannel::WaylandVirtualKeyboard
        } else if uinput_writable() {
            KeystrokeChannel::LinuxUinput
        } else {
            KeystrokeChannel::None
        };
        Self {
            keystroke,
            tracker: focus_tracker(),
        }
    }

    fn compositor_focus() -> CompositorFocus {
        if !super::hyprland::detected() {
            return CompositorFocus::Unavailable;
        }
        match super::hyprland::active_window() {
            Some(w) => CompositorFocus::Window { pid: w.pid },
            None => CompositorFocus::NoWindow,
        }
    }
}

/// uinput is opt-in: it counts only when the user has already granted device
/// access (udev rule / `input` group — see scripts/linux-inject-demo.sh). We
/// never escalate to get it.
fn uinput_writable() -> bool {
    rustix::fs::access("/dev/uinput", rustix::fs::Access::WRITE_OK).is_ok()
}

impl TextInjector for LinuxTextInjector {
    fn caps(&self) -> InjectorCaps {
        InjectorCaps {
            native_text_insert: false,
            keystroke: self.keystroke,
            clipboard: false,
        }
    }

    fn focused_field(&self) -> FieldKind {
        let compositor = Self::compositor_focus();
        let observed = self.tracker.and_then(|tracker| {
            tracker
                .latest
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().map(|t| t.obs))
        });
        // No event vouches for the focused window: ask its accessibility tree
        // directly (#8 — a password field focused before launch).
        let obs = match (lookup_pid(observed, compositor), self.tracker) {
            (Some(pid), Some(tracker)) => lookup_focused(tracker, pid).or(observed),
            _ => observed,
        };
        classify_focus(obs, compositor)
    }

    fn insert_native(&mut self, _text: &str) -> Result<(), InjectError> {
        Err(InjectError(
            "AT-SPI native insert is detection-only on Linux (ADR-0013 amendment)".into(),
        ))
    }

    fn synth_text(&mut self, text: &str) -> Result<(), InjectError> {
        match self.keystroke {
            KeystrokeChannel::WaylandVirtualKeyboard => {
                let mut kb = super::wayland_vk::VirtualKeyboard::connect()
                    .map_err(|e| InjectError(e.to_string()))?;
                // P9 during delivery: pin the window that holds focus now and
                // stop typing the moment another one takes it (the text stays
                // in history). Without compositor focus there is nothing to pin.
                let target = super::hyprland::detected()
                    .then(super::hyprland::active_window)
                    .flatten()
                    .map(|w| w.address);
                let mut still_focused = || match &target {
                    Some(address) => {
                        super::hyprland::active_window().is_some_and(|w| &w.address == address)
                    }
                    None => true,
                };
                kb.type_text_guarded(text, &mut still_focused)
                    .map(|_| ())
                    .map_err(|e| InjectError(e.to_string()))
            }
            KeystrokeChannel::LinuxUinput => {
                let typable = super::uinput::typable_count(text);
                let total = text.chars().count();
                if typable != total {
                    // Never deliver a partial sentence: hold it instead (the
                    // text stays in history; the HUD offers copy).
                    return Err(InjectError(format!(
                        "uinput can type {typable} of {total} characters (US-QWERTY ASCII only)"
                    )));
                }
                let mut kb = super::uinput::UinputKeyboard::open()
                    .map_err(|e| InjectError(format!("uinput: {e}")))?;
                kb.type_text(text)
                    .map(|_| ())
                    .map_err(|e| InjectError(format!("uinput: {e}")))
            }
            _ => Err(InjectError("no Linux keystroke channel available".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_role_is_secure_even_if_editable() {
        assert_eq!(
            role_to_field_kind(atspi::Role::PasswordText, true),
            FieldKind::Secure
        );
    }

    #[test]
    fn editable_entry_is_a_target() {
        assert_eq!(
            role_to_field_kind(atspi::Role::Entry, true),
            FieldKind::Editable
        );
    }

    #[test]
    fn non_editable_is_no_target() {
        assert_eq!(
            role_to_field_kind(atspi::Role::Label, false),
            FieldKind::NoTarget
        );
    }

    fn obs(kind: FieldKind, pid: u32) -> Option<FocusObservation> {
        Some(FocusObservation {
            kind,
            pid: Some(pid),
        })
    }

    #[test]
    fn same_process_trusts_the_accessibility_verdict() {
        let window = CompositorFocus::Window { pid: Some(42) };
        assert_eq!(
            classify_focus(obs(FieldKind::Editable, 42), window),
            FieldKind::Editable
        );
        assert_eq!(
            classify_focus(obs(FieldKind::Secure, 42), window),
            FieldKind::Secure
        );
        assert_eq!(
            classify_focus(obs(FieldKind::NoTarget, 42), window),
            FieldKind::NoTarget
        );
    }

    #[test]
    fn stale_editable_from_another_app_cannot_vouch_for_the_focused_window() {
        // User typed in gedit (pid 42), then switched to an opaque terminal (7).
        assert_eq!(
            classify_focus(
                obs(FieldKind::Editable, 42),
                CompositorFocus::Window { pid: Some(7) }
            ),
            FieldKind::Unknown
        );
    }

    #[test]
    fn opaque_or_pidless_focus_is_unknown() {
        assert_eq!(
            classify_focus(None, CompositorFocus::Window { pid: Some(7) }),
            FieldKind::Unknown
        );
        assert_eq!(
            classify_focus(
                obs(FieldKind::Editable, 7),
                CompositorFocus::Window { pid: None }
            ),
            FieldKind::Unknown
        );
        assert_eq!(
            classify_focus(obs(FieldKind::Editable, 7), CompositorFocus::NoWindow),
            FieldKind::Unknown
        );
    }

    #[test]
    fn active_lookup_runs_only_when_events_cannot_vouch() {
        let window = CompositorFocus::Window { pid: Some(42) };
        // No observation at all (field focused before the tracker started).
        assert_eq!(lookup_pid(None, window), Some(42));
        // Observation from another process (stale).
        assert_eq!(lookup_pid(obs(FieldKind::Editable, 7), window), Some(42));
        // A pid-less observation cannot vouch either.
        let pidless = Some(FocusObservation {
            kind: FieldKind::Editable,
            pid: None,
        });
        assert_eq!(lookup_pid(pidless, window), Some(42));
        // A fresh observation from the focused process is trusted as-is.
        assert_eq!(lookup_pid(obs(FieldKind::Secure, 42), window), None);
        // Nothing to search without a compositor-known pid.
        assert_eq!(
            lookup_pid(None, CompositorFocus::Window { pid: None }),
            None
        );
        assert_eq!(lookup_pid(None, CompositorFocus::NoWindow), None);
        assert_eq!(lookup_pid(None, CompositorFocus::Unavailable), None);
    }

    #[test]
    fn without_compositor_truth_only_secure_is_honoured() {
        // GNOME/KDE Wayland: a Secure observation still refuses (fail-safe)…
        assert_eq!(
            classify_focus(obs(FieldKind::Secure, 1), CompositorFocus::Unavailable),
            FieldKind::Secure
        );
        // …but an Editable one cannot upgrade an unverifiable target.
        assert_eq!(
            classify_focus(obs(FieldKind::Editable, 1), CompositorFocus::Unavailable),
            FieldKind::Unknown
        );
        assert_eq!(
            classify_focus(None, CompositorFocus::Unavailable),
            FieldKind::Unknown
        );
    }
}
