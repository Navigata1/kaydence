//! Wayland virtual-keyboard keystroke channel (ADR-0013 ladder rung 2,
//! ADR-0023) — `zwp_virtual_keyboard_v1`, implemented by Hyprland (Omarchy),
//! Sway, and every wlroots compositor.
//!
//! Why this is the Omarchy primary path:
//! - **Rootless.** No `/dev/uinput` device permission, no udev rule, no daemon.
//!   (uinput stays as the explicit opt-in fallback for compositors without this
//!   protocol, e.g. GNOME/Mutter.)
//! - **Full Unicode, layout-independent.** We upload our own per-injection XKB
//!   keymap (`inject/xkb.rs`) in which each distinct character is one key, so
//!   emoji/CJK/RTL/combining marks type exactly — closing the "ASCII-only" gap
//!   the uinput backend carries.
//! - **Compositor-mediated.** Keys travel through the compositor's normal input
//!   path to whichever surface holds keyboard focus; the caller has already run
//!   the focus binding (Pitfall P9) and secure-field gate (#8) before we type.
//!
//! Privacy (#1): the keymap reveals which characters are typed, so it is written
//! to an anonymous sealed `memfd` — never a filesystem path — and the fd is
//! dropped as soon as the compositor has it. Nothing is persisted or transmitted.
#![cfg(target_os = "linux")]
#![allow(dead_code)]

use super::xkb::{plan_typing, MAX_KEYS_PER_KEYMAP};
use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

/// `wl_keyboard.keymap_format.xkb_v1`.
const KEYMAP_FORMAT_XKB_V1: u32 = 1;
const KEY_PRESSED: u32 = 1;
const KEY_RELEASED: u32 = 0;
/// Let clients apply a freshly uploaded keymap before the first key of a chunk.
/// Measured on Hyprland 0.56 (see ADR-0023 evidence); without a settle some
/// clients read the first key through their previous keymap.
const KEYMAP_SETTLE: Duration = Duration::from_millis(15);
/// Keys per guarded batch. Before each batch the delivery target is
/// re-checked (Pitfall P9 *during* delivery): if focus moved, typing stops, so
/// at most one batch can land in a window the user switched to mid-dictation.
/// Each batch ends with a round trip, so the compositor has *processed* it
/// before focus is checked again — a mere flush let three batches sit queued
/// in the compositor while focus moved (live-found 2026-09-26: 12 keys went
/// to the new window).
const GUARD_EVERY: usize = 4;

/// How a guarded delivery ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Every key was sent.
    Complete,
    /// The target lost focus; `sent` keys went out before the check caught it.
    FocusMoved { sent: usize },
}

/// Send `keys` in batches of `every`, asking `still_focused` before each batch;
/// `send` emits one batch and makes it visible to the compositor. Pure over its
/// closures, so the P9 guard is unit-tested without a compositor.
pub fn deliver_guarded<E>(
    keys: &[u32],
    every: usize,
    still_focused: &mut dyn FnMut() -> bool,
    send: &mut dyn FnMut(&[u32]) -> Result<(), E>,
) -> Result<Delivery, E> {
    let mut sent = 0;
    for batch in keys.chunks(every.max(1)) {
        if !still_focused() {
            return Ok(Delivery::FocusMoved { sent });
        }
        send(batch)?;
        sent += batch.len();
    }
    Ok(Delivery::Complete)
}

#[derive(Debug, thiserror::Error)]
pub enum VkError {
    #[error("no Wayland display in this session")]
    NoDisplay,
    #[error("the compositor does not offer {0}")]
    Unsupported(&'static str),
    #[error("Wayland protocol error: {0}")]
    Protocol(String),
    #[error("keymap memfd: {0}")]
    Keymap(String),
    #[error(
        "focus moved during delivery: typed {typed} of {total} characters; the rest was withheld"
    )]
    FocusMoved { typed: usize, total: usize },
}

/// Registry/queue state. The virtual keyboard objects emit no events; the seat
/// emits capabilities/name we do not need.
struct VkState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for VkState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(VkState: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(VkState: ignore ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(VkState: ignore ZwpVirtualKeyboardV1);

/// One live virtual keyboard bound to the default seat.
pub struct VirtualKeyboard {
    conn: Connection,
    queue: EventQueue<VkState>,
    seat: wl_seat::WlSeat,
    manager: ZwpVirtualKeyboardManagerV1,
    keyboard: ZwpVirtualKeyboardV1,
    epoch: Instant,
}

impl VirtualKeyboard {
    /// Connect to the session compositor and create a virtual keyboard. Cheap
    /// (~1 ms): done per injection so a compositor restart never leaves us with
    /// a dead connection.
    pub fn connect() -> Result<Self, VkError> {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(VkError::NoDisplay);
        }
        let conn = Connection::connect_to_env().map_err(|_| VkError::NoDisplay)?;
        let (globals, mut queue) =
            registry_queue_init::<VkState>(&conn).map_err(|e| VkError::Protocol(e.to_string()))?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| VkError::Unsupported("wl_seat"))?;
        let manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| VkError::Unsupported("zwp_virtual_keyboard_manager_v1"))?;
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        // A compositor that forbids virtual keyboards (e.g. Hyprland with
        // `ecosystem:enforce_permissions`) answers with a protocol error here.
        queue
            .roundtrip(&mut VkState)
            .map_err(|e| VkError::Protocol(e.to_string()))?;
        Ok(Self {
            conn,
            queue,
            seat,
            manager,
            keyboard,
            epoch: Instant::now(),
        })
    }

    /// Replace the virtual keyboard with a brand-new device. Hyprland forwards a
    /// virtual keyboard's keymap to the focused client only when that device
    /// becomes the active keyboard, so re-uploading a keymap on the SAME device
    /// leaves the client decoding with the old one (live-found: chunk 2 of a
    /// 300-character passage came out as chunk 1's characters). A fresh device
    /// per keymap forces the switch.
    fn renew_keyboard(&mut self) -> Result<(), VkError> {
        self.keyboard.destroy();
        let qh = self.queue.handle();
        self.keyboard = self.manager.create_virtual_keyboard(&self.seat, &qh, ());
        self.roundtrip()
    }

    /// True when this session can create a virtual keyboard (probe only; the
    /// keyboard is destroyed immediately).
    pub fn available() -> bool {
        Self::connect().is_ok()
    }

    fn now_ms(&self) -> u32 {
        (self.epoch.elapsed().as_millis() & u128::from(u32::MAX)) as u32
    }

    /// Type `text` into whatever surface has keyboard focus. Returns the number
    /// of characters typed; untypable control characters are reported as an
    /// error rather than silently dropped.
    pub fn type_text(&mut self, text: &str) -> Result<usize, VkError> {
        self.type_text_guarded(text, &mut || true)
    }

    /// [`Self::type_text`], re-checking `still_focused` every [`GUARD_EVERY`]
    /// keys and stopping with [`VkError::FocusMoved`] if the target lost focus.
    pub fn type_text_guarded(
        &mut self,
        text: &str,
        still_focused: &mut dyn FnMut() -> bool,
    ) -> Result<usize, VkError> {
        let plan = plan_typing(text, MAX_KEYS_PER_KEYMAP);
        if plan.skipped > 0 {
            return Err(VkError::Keymap(format!(
                "{} control character(s) cannot be typed",
                plan.skipped
            )));
        }
        let mut typed = 0;
        for (n, chunk) in plan.chunks.iter().enumerate() {
            if n > 0 {
                self.renew_keyboard()?;
            }
            self.upload_keymap(&chunk.keymap)?;
            self.keyboard.modifiers(0, 0, 0, 0);
            self.roundtrip()?;
            std::thread::sleep(KEYMAP_SETTLE);
            let (keyboard, queue, epoch) = (&self.keyboard, &mut self.queue, self.epoch);
            let outcome = deliver_guarded(&chunk.keys, GUARD_EVERY, still_focused, &mut |batch| {
                for key in batch {
                    let t = (epoch.elapsed().as_millis() & u128::from(u32::MAX)) as u32;
                    keyboard.key(t, *key, KEY_PRESSED);
                    keyboard.key(t, *key, KEY_RELEASED);
                }
                queue
                    .roundtrip(&mut VkState)
                    .map(|_| ())
                    .map_err(|e| VkError::Protocol(e.to_string()))
            })?;
            match outcome {
                Delivery::Complete => typed += chunk.keys.len(),
                Delivery::FocusMoved { sent } => {
                    let _ = self.roundtrip();
                    return Err(VkError::FocusMoved {
                        typed: typed + sent,
                        total: plan.key_count(),
                    });
                }
            }
            // Every key of this chunk must be delivered under THIS keymap before
            // the next chunk replaces it.
            self.roundtrip()?;
        }
        Ok(typed)
    }

    fn upload_keymap(&mut self, keymap: &str) -> Result<(), VkError> {
        let fd = rustix::fs::memfd_create(
            "keymap",
            rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
        )
        .map_err(|e| VkError::Keymap(e.to_string()))?;
        let mut file = std::fs::File::from(fd);
        // The protocol wants a NUL-terminated XKB string.
        file.write_all(keymap.as_bytes())
            .and_then(|_| file.write_all(&[0]))
            .map_err(|e| VkError::Keymap(e.to_string()))?;
        // Seal so the compositor can mmap it without fearing a later resize.
        let _ = rustix::fs::fcntl_add_seals(
            &file,
            rustix::fs::SealFlags::SHRINK
                | rustix::fs::SealFlags::GROW
                | rustix::fs::SealFlags::WRITE,
        );
        let size = u32::try_from(keymap.len() + 1)
            .map_err(|_| VkError::Keymap("keymap too large".into()))?;
        self.keyboard
            .keymap(KEYMAP_FORMAT_XKB_V1, file.as_fd(), size);
        // The request (with its fd) is queued; flush before `file` drops.
        self.conn
            .flush()
            .map_err(|e| VkError::Protocol(e.to_string()))
    }

    fn roundtrip(&mut self) -> Result<(), VkError> {
        self.queue
            .roundtrip(&mut VkState)
            .map(|_| ())
            .map_err(|e| VkError::Protocol(e.to_string()))
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        self.keyboard.destroy();
        let _ = self.conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        keys: &[u32],
        mut focus: impl FnMut(usize) -> bool,
    ) -> (Result<Delivery, &'static str>, Vec<Vec<u32>>) {
        let mut batches: Vec<Vec<u32>> = Vec::new();
        let mut checks = 0;
        let result = deliver_guarded(
            keys,
            GUARD_EVERY,
            &mut || {
                checks += 1;
                focus(checks)
            },
            &mut |batch| {
                batches.push(batch.to_vec());
                Ok(())
            },
        );
        (result, batches)
    }

    #[test]
    fn steady_focus_sends_every_key_in_small_batches() {
        let keys: Vec<u32> = (1..=10).collect();
        let (result, batches) = run(&keys, |_| true);
        assert_eq!(result, Ok(Delivery::Complete));
        assert_eq!(
            batches,
            vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![9, 10]]
        );
    }

    #[test]
    fn focus_change_mid_delivery_withholds_the_rest() {
        // Focus holds for the first two checks, then the user switches window.
        let keys: Vec<u32> = (1..=20).collect();
        let (result, batches) = run(&keys, |check| check <= 2);
        assert_eq!(result, Ok(Delivery::FocusMoved { sent: 8 }));
        assert_eq!(batches.concat(), (1..=8).collect::<Vec<u32>>());
    }

    #[test]
    fn focus_lost_before_the_first_key_sends_nothing() {
        let (result, batches) = run(&[1, 2, 3], |_| false);
        assert_eq!(result, Ok(Delivery::FocusMoved { sent: 0 }));
        assert!(batches.is_empty());
    }

    #[test]
    fn send_errors_propagate() {
        let result = deliver_guarded(&[1, 2, 3, 4, 5], 2, &mut || true, &mut |_| Err("boom"));
        assert_eq!(result, Err("boom"));
    }

    #[test]
    fn at_most_one_batch_can_leak_after_a_switch() {
        // Whatever the timing, keys sent after the last passing check are
        // bounded by the batch size.
        for switch_after in 0..6 {
            let keys: Vec<u32> = (1..=24).collect();
            let (result, _) = run(&keys, |check| check <= switch_after);
            let Ok(Delivery::FocusMoved { sent }) = result else {
                panic!("expected an interrupted delivery");
            };
            assert_eq!(sent, switch_after * GUARD_EVERY);
        }
    }
}
