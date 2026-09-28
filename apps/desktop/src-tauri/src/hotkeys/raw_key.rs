//! Edge detection for a bare Right-Alt hotkey fed from raw keyboard records
//! (Windows Raw Input, ADR-0022). Pure and platform-agnostic so the rules are
//! unit-tested on every OS; the Win32 listener only decodes records into
//! [`RawKey`] and acts on the returned [`RightAltEdge`].
//!
//! Privacy (ADR-0022 §5): Raw Input delivers every keystroke system-wide. Each
//! record is reduced here to a Right-Alt edge, the Shift state, or "another key
//! went down", and then dropped. Key identities never leave this module.

/// `VK_*` values (winuser.h) the reducer needs, kept raw so it stays pure.
pub mod vk {
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const LSHIFT: u16 = 0xA0;
    pub const RSHIFT: u16 = 0xA1;
    pub const RMENU: u16 = 0xA5;
    /// Unassigned. Injected while Right-Alt is held so its release does not
    /// activate the focused app's menu.
    pub const MENU_MASK: u16 = 0xE8;
    /// Raw Input's placeholder for fake or overrun records.
    pub const OVERRUN: u16 = 0xFF;
}

/// One decoded raw keyboard record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawKey {
    pub vkey: u16,
    /// The `E0` prefix: distinguishes Right-Alt from Left-Alt.
    pub extended: bool,
    pub key_up: bool,
    /// Carries Kaydence's own `dwExtraInfo` tag (the menu-mask keystroke).
    pub injected_by_us: bool,
}

/// What a Right-Alt hold means to the capture coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightAltEdge {
    /// First press of a hold. `shift` selects the cleanup-override role.
    Down {
        shift: bool,
    },
    Up,
    /// Another key went down during the hold (AltGr text, Alt+Tab): the hold
    /// is a chord, not push-to-talk. Emitted at most once per hold.
    Chord,
}

#[derive(Debug, Default)]
pub struct RightAltTracker {
    held: bool,
    chorded: bool,
    shift_down: bool,
}

impl RightAltTracker {
    pub fn feed(&mut self, key: RawKey) -> Option<RightAltEdge> {
        if key.injected_by_us || matches!(key.vkey, vk::MENU_MASK | vk::OVERRUN) {
            return None;
        }
        if is_shift(key.vkey) {
            self.shift_down = !key.key_up;
            return None;
        }
        if is_right_alt(key) {
            return self.right_alt(key.key_up);
        }
        if self.held && !key.key_up && !self.chorded && !is_modifier(key.vkey) {
            self.chorded = true;
            return Some(RightAltEdge::Chord);
        }
        None
    }

    fn right_alt(&mut self, key_up: bool) -> Option<RightAltEdge> {
        match (self.held, key_up) {
            (false, false) => {
                self.held = true;
                self.chorded = false;
                Some(RightAltEdge::Down {
                    shift: self.shift_down,
                })
            }
            // Typematic auto-repeat while held.
            (true, false) => None,
            (true, true) => {
                self.held = false;
                Some(RightAltEdge::Up)
            }
            // Release of a press that began before the listener started.
            (false, true) => None,
        }
    }
}

fn is_right_alt(key: RawKey) -> bool {
    (key.vkey == vk::MENU && key.extended) || key.vkey == vk::RMENU
}

fn is_shift(vkey: u16) -> bool {
    matches!(vkey, vk::SHIFT | vk::LSHIFT | vk::RSHIFT)
}

fn is_modifier(vkey: u16) -> bool {
    matches!(
        vkey,
        vk::SHIFT | vk::CONTROL | vk::MENU | vk::LWIN | vk::RWIN | 0xA0..=0xA5
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(vkey: u16, extended: bool, key_up: bool) -> RawKey {
        RawKey {
            vkey,
            extended,
            key_up,
            injected_by_us: false,
        }
    }

    fn right_alt(key_up: bool) -> RawKey {
        key(vk::MENU, true, key_up)
    }

    fn feed_all(tracker: &mut RightAltTracker, keys: &[RawKey]) -> Vec<RightAltEdge> {
        keys.iter().filter_map(|k| tracker.feed(*k)).collect()
    }

    #[test]
    fn hold_emits_down_then_up_and_ignores_auto_repeat() {
        let mut t = RightAltTracker::default();
        let edges = feed_all(
            &mut t,
            &[
                right_alt(false),
                right_alt(false),
                right_alt(false),
                right_alt(true),
            ],
        );
        assert_eq!(
            edges,
            vec![RightAltEdge::Down { shift: false }, RightAltEdge::Up]
        );
    }

    #[test]
    fn left_alt_is_not_the_hotkey() {
        let mut t = RightAltTracker::default();
        assert!(feed_all(
            &mut t,
            &[key(vk::MENU, false, false), key(vk::MENU, false, true)]
        )
        .is_empty());
    }

    #[test]
    fn shift_held_before_press_selects_the_override_role() {
        let mut t = RightAltTracker::default();
        let edges = feed_all(
            &mut t,
            &[
                key(vk::SHIFT, false, false),
                right_alt(false),
                right_alt(true),
            ],
        );
        assert_eq!(
            edges,
            vec![RightAltEdge::Down { shift: true }, RightAltEdge::Up]
        );
        t.feed(key(vk::SHIFT, false, true));
        assert_eq!(
            t.feed(right_alt(false)),
            Some(RightAltEdge::Down { shift: false })
        );
    }

    #[test]
    fn another_key_during_the_hold_is_a_chord_once() {
        let mut t = RightAltTracker::default();
        let edges = feed_all(
            &mut t,
            &[
                right_alt(false),
                key(0x09, false, false), // Tab (Alt+Tab)
                key(0x09, false, true),
                key(0x09, false, false),
                right_alt(true),
            ],
        );
        assert_eq!(
            edges,
            vec![
                RightAltEdge::Down { shift: false },
                RightAltEdge::Chord,
                RightAltEdge::Up
            ]
        );
    }

    #[test]
    fn altgr_character_sequence_is_a_chord() {
        // AltGr layouts: fake Left-Ctrl down, Right-Alt down, 'Q', releases.
        let mut t = RightAltTracker::default();
        let edges = feed_all(
            &mut t,
            &[
                key(vk::CONTROL, false, false),
                right_alt(false),
                key(0x51, false, false),
                key(0x51, false, true),
                key(vk::CONTROL, false, true),
                right_alt(true),
            ],
        );
        assert_eq!(
            edges,
            vec![
                RightAltEdge::Down { shift: false },
                RightAltEdge::Chord,
                RightAltEdge::Up
            ]
        );
    }

    #[test]
    fn modifiers_and_key_ups_during_the_hold_are_not_chords() {
        let mut t = RightAltTracker::default();
        let edges = feed_all(
            &mut t,
            &[
                right_alt(false),
                key(vk::CONTROL, false, false),
                key(vk::LWIN, true, false),
                key(0x41, false, true), // stray release of a key pressed earlier
                right_alt(true),
            ],
        );
        assert_eq!(
            edges,
            vec![RightAltEdge::Down { shift: false }, RightAltEdge::Up]
        );
    }

    #[test]
    fn our_mask_keystroke_and_overrun_records_are_ignored() {
        let mut t = RightAltTracker::default();
        let tagged = RawKey {
            injected_by_us: true,
            ..key(0x41, false, false)
        };
        let edges = feed_all(
            &mut t,
            &[
                right_alt(false),
                key(vk::MENU_MASK, false, false),
                key(vk::MENU_MASK, false, true),
                tagged,
                key(vk::OVERRUN, false, false),
                right_alt(true),
            ],
        );
        assert_eq!(
            edges,
            vec![RightAltEdge::Down { shift: false }, RightAltEdge::Up]
        );
    }

    #[test]
    fn release_without_a_seen_press_is_ignored() {
        let mut t = RightAltTracker::default();
        assert_eq!(t.feed(right_alt(true)), None);
        assert_eq!(
            t.feed(right_alt(false)),
            Some(RightAltEdge::Down { shift: false })
        );
    }

    #[test]
    fn rmenu_vkey_is_also_right_alt() {
        let mut t = RightAltTracker::default();
        assert_eq!(
            t.feed(key(vk::RMENU, false, false)),
            Some(RightAltEdge::Down { shift: false })
        );
    }
}
