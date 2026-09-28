//! Windows Raw Input listener for the bare Right-Alt hotkey (ADR-0022).
//!
//! `global-hotkey` cannot register a bare Right-Alt on Windows, and
//! `RegisterHotKey(0, VK_RMENU)` registers but never fires. This listener owns
//! one hidden, never-shown `STATIC` window registered for keyboard Raw Input
//! with `RIDEV_INPUTSINK` (background delivery, no hook, no elevation) and
//! handles `WM_INPUT` in its own message loop.
//!
//! Records go through the pure [`RightAltTracker`]; only edges leave this
//! module (ADR-0022 §5). While Kaydence owns a press, an unassigned key is
//! injected so the release does not activate the focused app's menu. The input
//! thread never runs capture work: edges go to a worker thread, so masking is
//! immediate and the message loop is never re-entered.
#![cfg(target_os = "windows")]

use std::sync::{mpsc, Arc};
use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY,
};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetMessageW, MSG, RI_KEY_BREAK, RI_KEY_E0, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_INPUT,
};

use super::raw_key::{vk, RawKey, RightAltEdge, RightAltTracker};

/// `dwExtraInfo` tag on Kaydence's own mask keystrokes ("KAYD"); fits the
/// 32-bit `RAWKEYBOARD::ExtraInformation` it comes back in.
const KAYDENCE_INPUT_TAG: u32 = 0x4B41_5944;
/// HID Generic Desktop page / Keyboard usage.
const HID_USAGE_PAGE_GENERIC: u16 = 0x01;
const HID_USAGE_KEYBOARD: u16 = 0x06;

/// The app side of the listener.
pub trait RightAltSink: Send + Sync + 'static {
    /// Fast and non-blocking: does Kaydence own this press right now?
    /// Decides masking and delivery for the whole hold.
    fn owns(&self, shift: bool) -> bool;
    /// Deliver an owned edge with its receipt time. Runs on a worker thread
    /// and may do capture work.
    fn deliver(&self, edge: RightAltEdge, at: Instant);
}

#[derive(Debug, thiserror::Error)]
pub enum RawHotkeyError {
    #[error("Right-Alt Raw Input listener setup failed: {0}")]
    Setup(String),
}

/// Start the listener. Returns once Raw Input registration succeeded or
/// failed, so a failure surfaces as a hotkey registration error (invariant 3).
pub fn spawn_right_alt_listener(sink: Arc<dyn RightAltSink>) -> Result<(), RawHotkeyError> {
    let (edge_tx, edge_rx) = mpsc::channel::<(RightAltEdge, Instant)>();
    let worker_sink = Arc::clone(&sink);
    std::thread::Builder::new()
        .name("kaydence-right-alt-dispatch".into())
        .spawn(move || {
            for (edge, at) in edge_rx {
                worker_sink.deliver(edge, at);
            }
        })
        .map_err(|err| RawHotkeyError::Setup(err.to_string()))?;

    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kaydence-right-alt-input".into())
        .spawn(move || match create_raw_input_target() {
            Ok(hwnd) => {
                let _ = ready_tx.send(Ok(()));
                pump(hwnd, sink, edge_tx);
            }
            Err(err) => {
                let _ = ready_tx.send(Err(err));
            }
        })
        .map_err(|err| RawHotkeyError::Setup(err.to_string()))?;

    ready_rx
        .recv()
        .map_err(|_| RawHotkeyError::Setup("listener thread exited during setup".into()))?
}

fn create_raw_input_target() -> Result<HWND, RawHotkeyError> {
    // Built-in class, never shown: no class registration, no taskbar entry.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )
    }
    .map_err(|err| RawHotkeyError::Setup(format!("hidden window: {err}")))?;

    let device = RAWINPUTDEVICE {
        usUsagePage: HID_USAGE_PAGE_GENERIC,
        usUsage: HID_USAGE_KEYBOARD,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    };
    unsafe { RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32) }
        .map_err(|err| RawHotkeyError::Setup(format!("RegisterRawInputDevices: {err}")))?;
    Ok(hwnd)
}

/// Runs for the life of the process on the input thread.
fn pump(hwnd: HWND, sink: Arc<dyn RightAltSink>, edges: mpsc::Sender<(RightAltEdge, Instant)>) {
    let mut tracker = RightAltTracker::default();
    let mut owned_hold = false;
    let mut msg = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut msg, Some(hwnd), 0, 0) }.0;
        if status == 0 || status == -1 {
            eprintln!("Kaydence Right-Alt listener stopped (GetMessageW returned {status})");
            return;
        }
        if msg.message == WM_INPUT {
            let at = Instant::now();
            if let Some(key) = read_raw_key(msg.lParam) {
                if route(tracker.feed(key), at, &mut owned_hold, &*sink, &edges) {
                    send_menu_mask();
                }
            }
        }
        // WM_INPUT still needs DefWindowProc cleanup; the STATIC proc calls it.
        unsafe { DispatchMessageW(&msg) };
    }
}

/// Forward an edge of an owned hold to the worker. Returns true when the
/// caller must inject the menu mask (the first edge of an owned hold).
fn route(
    edge: Option<RightAltEdge>,
    at: Instant,
    owned_hold: &mut bool,
    sink: &dyn RightAltSink,
    edges: &mpsc::Sender<(RightAltEdge, Instant)>,
) -> bool {
    let Some(edge) = edge else {
        return false;
    };
    let mut mask = false;
    if let RightAltEdge::Down { shift } = edge {
        *owned_hold = sink.owns(shift);
        mask = *owned_hold;
    }
    if *owned_hold {
        let _ = edges.send((edge, at));
    }
    if edge == RightAltEdge::Up {
        *owned_hold = false;
    }
    mask
}

/// Decode one `WM_INPUT` record. Only the fields the reducer needs are read;
/// nothing is retained.
fn read_raw_key(lparam: LPARAM) -> Option<RawKey> {
    let mut raw = RAWINPUT::default();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    let read = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam.0 as *mut _),
            RID_INPUT,
            Some(&mut raw as *mut RAWINPUT as *mut _),
            &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if read == 0 || read == u32::MAX || raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return None;
    }
    let keyboard = unsafe { raw.data.keyboard };
    let flags = u32::from(keyboard.Flags);
    Some(RawKey {
        vkey: keyboard.VKey,
        extended: flags & RI_KEY_E0 != 0,
        key_up: flags & RI_KEY_BREAK != 0,
        injected_by_us: keyboard.ExtraInformation == KAYDENCE_INPUT_TAG,
    })
}

/// Inject the unassigned mask key so releasing Right-Alt does not activate
/// the focused app's menu. UIPI blocks this for elevated foreground apps.
fn send_menu_mask() {
    let inputs = [mask_input(false), mask_input(true)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        eprintln!(
            "Kaydence Right-Alt menu mask was blocked ({sent} of {} events; elevated foreground app?)",
            inputs.len()
        );
    }
}

fn mask_input(key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk::MENU_MASK),
                wScan: 0,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: KAYDENCE_INPUT_TAG as usize,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedSink {
        owns: bool,
    }

    impl RightAltSink for FixedSink {
        fn owns(&self, _shift: bool) -> bool {
            self.owns
        }
        fn deliver(&self, _edge: RightAltEdge, _at: Instant) {}
    }

    /// Route a script; return what reaches the worker and each mask decision.
    fn run(owns: bool, script: &[RightAltEdge]) -> (Vec<RightAltEdge>, Vec<bool>) {
        let sink = FixedSink { owns };
        let (tx, rx) = mpsc::channel();
        let mut owned = false;
        let masks = script
            .iter()
            .map(|edge| route(Some(*edge), Instant::now(), &mut owned, &sink, &tx))
            .collect();
        drop(tx);
        (rx.into_iter().map(|(edge, _)| edge).collect(), masks)
    }

    #[test]
    fn unowned_holds_are_never_forwarded_or_masked() {
        let script = [
            RightAltEdge::Down { shift: false },
            RightAltEdge::Chord,
            RightAltEdge::Up,
        ];
        let (forwarded, masks) = run(false, &script);
        assert!(forwarded.is_empty());
        assert_eq!(masks, vec![false, false, false]);
    }

    #[test]
    fn owned_holds_mask_once_forward_every_edge_then_reset() {
        let script = [
            RightAltEdge::Down { shift: true },
            RightAltEdge::Chord,
            RightAltEdge::Up,
            RightAltEdge::Up, // stray: hold already closed
        ];
        let (forwarded, masks) = run(true, &script);
        assert_eq!(forwarded, script[..3].to_vec());
        assert_eq!(masks, vec![true, false, false, false]);
    }

    #[test]
    fn mask_keystroke_carries_the_ignore_tag() {
        let down = mask_input(false);
        let up = mask_input(true);
        unsafe {
            assert_eq!(down.Anonymous.ki.wVk, VIRTUAL_KEY(vk::MENU_MASK));
            assert_eq!(down.Anonymous.ki.dwExtraInfo, KAYDENCE_INPUT_TAG as usize);
            assert_eq!(up.Anonymous.ki.dwFlags, KEYEVENTF_KEYUP);
        }
    }
}
