//! Wayland/Hyprland injection self-test (ADR-0023) — the Omarchy twin of
//! `atspi-selftest`. Linux-only; a no-op elsewhere.
//!
//!   wayland-selftest --probe
//!       Print the Hyprland focused window + whether a virtual keyboard can be
//!       created in this session.
//!
//!   wayland-selftest --type-into <hyprland-window-address> <text>
//!       Focus that window through Hyprland IPC, RE-VERIFY it is the focused
//!       window (the Pitfall-P9 focus gate — if anything else has focus we
//!       refuse to type), type <text> through `zwp_virtual_keyboard_v1`, then
//!       hand focus back to the window that had it before. Harness: see
//!       `scripts/linux-wayland-inject-proof.sh`.

fn main() {
    #[cfg(target_os = "linux")]
    std::process::exit(linux::run(std::env::args().collect()));
    #[cfg(not(target_os = "linux"))]
    eprintln!("wayland-selftest is Linux-only.");
}

#[cfg(target_os = "linux")]
mod linux {
    use kaydence_lib::inject::hyprland;
    use kaydence_lib::inject::wayland_vk::VirtualKeyboard;
    use std::time::{Duration, Instant};

    /// How long the target keeps focus after the last key before the selftest
    /// hands focus back (see `type_into`).
    const CLIENT_SETTLE: Duration = Duration::from_millis(600);

    pub fn run(args: Vec<String>) -> i32 {
        match args.get(1).map(String::as_str) {
            Some("--probe") => probe(),
            Some("--type-into") if args.len() == 4 => type_into(&args[2], &args[3]),
            Some("--guarded-type-into") if args.len() == 4 => guarded_type_into(&args[2], &args[3]),
            _ => {
                eprintln!(
                    "usage: wayland-selftest --probe | --type-into <window-address> <text> \
                     | --guarded-type-into <window-address> <text>"
                );
                64
            }
        }
    }

    /// The SHIPPED delivery path, not just the keyboard: `LinuxTextInjector`
    /// (AT-SPI focus tracker + Hyprland pid match) → `inject_committed_text`
    /// (secure-field policy → plan → virtual keyboard). Exit 0 = Injected,
    /// 10 = Held{SecureField} (refused, nothing typed), 3 = focus gate refused,
    /// 1 = anything else.
    fn guarded_type_into(address: &str, text: &str) -> i32 {
        use kaydence_lib::events::{HoldReason, SessionEvent, SessionId};
        use kaydence_lib::inject::linux::LinuxTextInjector;
        use kaydence_lib::inject::{inject_committed_text, TextInjector, UnknownFieldPolicy};

        if !address.starts_with("0x") || !address[2..].chars().all(|c| c.is_ascii_hexdigit()) {
            eprintln!("[guard] refusing: {address:?} is not a Hyprland window address");
            return 64;
        }
        // Start the injector (and its AT-SPI focus tracker) BEFORE focus moves,
        // so the tracker sees the target's focus event like the running app would.
        let mut injector = LinuxTextInjector::detect();
        std::thread::sleep(Duration::from_millis(700));
        let previous = focused_address();
        focus_window(address);
        let deadline = Instant::now() + Duration::from_millis(1500);
        while focused_address().as_deref() != Some(address) {
            if Instant::now() > deadline {
                eprintln!("[guard] REFUSED: target never took focus — nothing typed (P9 gate)");
                return 3;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Let the accessibility focus event reach the tracker.
        std::thread::sleep(Duration::from_millis(600));
        let field = injector.focused_field();
        println!(
            "[guard] keystroke channel={:?} focused_field={field:?}",
            injector.caps().keystroke
        );
        let event = if focused_address().as_deref() == Some(address) {
            inject_committed_text(
                &mut injector,
                SessionId(ulid::Ulid::new()),
                text,
                UnknownFieldPolicy::Lenient,
                false,
            )
        } else {
            eprintln!("[guard] REFUSED: focus moved before delivery — nothing typed (P9 gate)");
            return 3;
        };
        std::thread::sleep(CLIENT_SETTLE);
        if let Some(prev) = previous.filter(|p| p != address) {
            focus_window(&prev);
        }
        println!("[guard] outcome={event:?}");
        match event {
            SessionEvent::Injected { .. } => 0,
            SessionEvent::Held {
                reason: HoldReason::SecureField,
                ..
            } => 10,
            _ => 1,
        }
    }

    fn probe() -> i32 {
        println!("[probe] hyprland session: {}", hyprland::detected());
        match hyprland::active_window() {
            Some(w) => println!(
                "[probe] focused window: class={:?} pid={:?} xwayland={} app_ref={:?}",
                w.class,
                w.pid,
                w.xwayland,
                w.app_ref()
            ),
            None => println!("[probe] focused window: none reported"),
        }
        let t = Instant::now();
        match VirtualKeyboard::connect() {
            Ok(_) => {
                println!(
                    "[probe] zwp_virtual_keyboard_v1: AVAILABLE (connect+create {} µs)",
                    t.elapsed().as_micros()
                );
                0
            }
            Err(e) => {
                println!("[probe] zwp_virtual_keyboard_v1: unavailable — {e}");
                1
            }
        }
    }

    fn focused_address() -> Option<String> {
        hyprland::active_window().map(|w| w.address)
    }

    /// Ask Hyprland to focus a window. Hyprland ≥ 0.55 in Lua-config mode (Omarchy
    /// 4) evaluates `dispatch` arguments as Lua; older/hyprlang sessions take the
    /// legacy dispatcher string. Try Lua first, as `omarchy-launch-or-focus` does.
    fn focus_window(address: &str) {
        let lua = format!("dispatch hl.dsp.focus({{ window = \"address:{address}\" }})");
        let ok = hyprland::request(&lua).is_some_and(|reply| reply.trim() == "ok");
        if !ok {
            let _ = hyprland::request(&format!("dispatch focuswindow address:{address}"));
        }
    }

    fn type_into(address: &str, text: &str) -> i32 {
        if !address.starts_with("0x") || !address[2..].chars().all(|c| c.is_ascii_hexdigit()) {
            eprintln!("[type] refusing: {address:?} is not a Hyprland window address");
            return 64;
        }
        let previous = focused_address();
        focus_window(address);

        // Wait (bounded) for the compositor to report the target as focused.
        let deadline = Instant::now() + Duration::from_millis(1500);
        while focused_address().as_deref() != Some(address) {
            if Instant::now() > deadline {
                eprintln!("[type] REFUSED: target never took focus — nothing typed (P9 gate)");
                return 3;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        let mut kb = match VirtualKeyboard::connect() {
            Ok(kb) => kb,
            Err(e) => {
                eprintln!("[type] virtual keyboard unavailable: {e}");
                return 1;
            }
        };
        // Final gate immediately before the first key.
        if focused_address().as_deref() != Some(address) {
            eprintln!("[type] REFUSED: focus moved before typing — nothing typed (P9 gate)");
            return 3;
        }
        let started = Instant::now();
        let result = kb.type_text(text);
        let elapsed = started.elapsed();
        drop(kb);

        // Let the client drain its key queue before focus leaves: GTK4 routes
        // keys through its input-method layer and drops in-flight events on
        // focus-out (live-found 2026-09-26: 16 of 28 chars landed without this).
        std::thread::sleep(CLIENT_SETTLE);
        if let Some(prev) = previous.filter(|p| p != address) {
            focus_window(&prev);
        }
        match result {
            Ok(n) => {
                println!(
                    "[type] typed {n} chars ({} bytes) into {address} in {} ms via zwp_virtual_keyboard_v1",
                    text.len(),
                    elapsed.as_millis()
                );
                0
            }
            Err(e) => {
                eprintln!("[type] FAILED: {e}");
                1
            }
        }
    }
}
