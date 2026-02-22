//! Keystroke classification and re-encoding for the Zellij plugin.
//!
//! Two public functions are provided:
//!
//! - [`classify`] — converts a `KeyWithModifier` into a semantic
//!   [`KeystrokeEvent`] understood by `crumbeez-lib`.
//! - [`key_to_bytes`] — converts a `KeyWithModifier` back into the raw VT/ANSI
//!   byte sequence that should be written to a terminal's stdin so the
//!   keystroke reaches the application running in the pane.
//!
//! Classification rules (in precedence order):
//!
//! 1. **Shortcut** — any key chord that has Ctrl, Alt, or Super held.
//!    Shift alone does *not* make a chord a shortcut (it just produces an
//!    upper-case character or a shifted navigation move).
//!
//! 2. **Navigation** — arrow keys, Home, End, PageUp, PageDown (with or
//!    without Shift/Ctrl held, since those are selection / word-jump moves
//!    that are still navigation, not shortcuts).
//!
//! 3. **Edit control** — Enter, Tab, Backspace, Delete, Insert (no
//!    Ctrl/Alt/Super — those fall into Shortcut).
//!
//! 4. **Escape** — Esc alone.
//!
//! 5. **Function key** — F1–F12 with no modifier at all.
//!
//! 6. **Text typed** — Char(_) with no Ctrl/Alt/Super.
//!
//! 7. **System key** — CapsLock, ScrollLock, NumLock, PrintScreen, Pause, Menu.

use zellij_tile::prelude::{BareKey, KeyModifier, KeyWithModifier};

use crumbeez_lib::{
    EditControlEvent, KeystrokeEvent, NavDirection, NavigationEvent, ShortcutEvent, ShortcutKey,
    SystemKeyEvent,
};

/// Classify a single [`KeyWithModifier`] into a [`KeystrokeEvent`].
pub fn classify(key: &KeyWithModifier) -> KeystrokeEvent {
    let ctrl = key.key_modifiers.contains(&KeyModifier::Ctrl);
    let alt = key.key_modifiers.contains(&KeyModifier::Alt);
    let shift = key.key_modifiers.contains(&KeyModifier::Shift);
    let super_key = key.key_modifiers.contains(&KeyModifier::Super);

    let is_chord = ctrl || alt || super_key;

    // ── 1. Shortcut ──────────────────────────────────────────────
    if is_chord {
        let sk = bare_key_to_shortcut_key(&key.bare_key);
        return KeystrokeEvent::Shortcut(ShortcutEvent {
            key: sk,
            ctrl,
            alt,
            shift,
            super_key,
        });
    }

    // ── 2. Navigation ────────────────────────────────────────────
    if let Some(dir) = nav_direction(&key.bare_key) {
        return KeystrokeEvent::Navigation(NavigationEvent {
            direction: dir,
            count: 1,
            with_shift: shift,
            with_ctrl: false, // ctrl already handled as chord above
        });
    }

    // ── 3. Edit control ──────────────────────────────────────────
    match key.bare_key {
        BareKey::Enter => return KeystrokeEvent::EditControl(EditControlEvent::Enter),
        BareKey::Tab => return KeystrokeEvent::EditControl(EditControlEvent::Tab),
        BareKey::Backspace => {
            return KeystrokeEvent::EditControl(EditControlEvent::Backspace { count: 1 })
        }
        BareKey::Delete => {
            return KeystrokeEvent::EditControl(EditControlEvent::Delete { count: 1 })
        }
        BareKey::Insert => return KeystrokeEvent::EditControl(EditControlEvent::Insert),
        _ => {}
    }

    // ── 4. Escape ────────────────────────────────────────────────
    if key.bare_key == BareKey::Esc {
        return KeystrokeEvent::Escape;
    }

    // ── 5. Function key (unmodified) ─────────────────────────────
    if let BareKey::F(n) = key.bare_key {
        return KeystrokeEvent::FunctionKey(n);
    }

    // ── 6. Text typed ────────────────────────────────────────────
    if let BareKey::Char(c) = key.bare_key {
        return KeystrokeEvent::TextTyped(c.to_string());
    }

    // ── 7. System keys ───────────────────────────────────────────
    let sys = match key.bare_key {
        BareKey::CapsLock => Some(SystemKeyEvent::CapsLock),
        BareKey::ScrollLock => Some(SystemKeyEvent::ScrollLock),
        BareKey::NumLock => Some(SystemKeyEvent::NumLock),
        BareKey::PrintScreen => Some(SystemKeyEvent::PrintScreen),
        BareKey::Pause => Some(SystemKeyEvent::Pause),
        BareKey::Menu => Some(SystemKeyEvent::Menu),
        _ => None,
    };
    if let Some(sys) = sys {
        return KeystrokeEvent::SystemKey(sys);
    }

    // Fallback: treat anything else as a shortcut with no modifiers so we
    // don't silently drop unknown keys.
    KeystrokeEvent::Shortcut(ShortcutEvent {
        key: bare_key_to_shortcut_key(&key.bare_key),
        ctrl: false,
        alt: false,
        shift,
        super_key: false,
    })
}

// ── Helpers ──────────────────────────────────────────────────────

fn nav_direction(bare: &BareKey) -> Option<NavDirection> {
    match bare {
        BareKey::Left => Some(NavDirection::Left),
        BareKey::Right => Some(NavDirection::Right),
        BareKey::Up => Some(NavDirection::Up),
        BareKey::Down => Some(NavDirection::Down),
        BareKey::Home => Some(NavDirection::Home),
        BareKey::End => Some(NavDirection::End),
        BareKey::PageUp => Some(NavDirection::PageUp),
        BareKey::PageDown => Some(NavDirection::PageDown),
        _ => None,
    }
}

// ── key_to_bytes ─────────────────────────────────────────────────

/// Encode a [`KeyWithModifier`] as the VT/ANSI byte sequence that a terminal
/// application expects to receive on its stdin.
///
/// This is the inverse of what a terminal emulator does when it translates a
/// physical keypress into an escape sequence.  We need it because
/// `intercept_key_presses()` redirects input *away* from the focused pane; we
/// must write the bytes back ourselves so the user's input is not swallowed.
///
/// Reference: XTerm Control Sequences, ECMA-48, and the Kitty keyboard
/// protocol (for the subset Zellij exposes).
pub fn key_to_bytes(key: &KeyWithModifier) -> Vec<u8> {
    let ctrl = key.key_modifiers.contains(&KeyModifier::Ctrl);
    let alt = key.key_modifiers.contains(&KeyModifier::Alt);
    let shift = key.key_modifiers.contains(&KeyModifier::Shift);

    // Alt prefix: ESC byte prepended to whatever the bare key produces.
    // We compute the inner sequence first and then wrap if Alt is set.
    let inner = bare_key_to_bytes(&key.bare_key, ctrl, shift);

    if alt && !inner.is_empty() {
        let mut out = Vec::with_capacity(1 + inner.len());
        out.push(0x1b); // ESC
        out.extend_from_slice(&inner);
        out
    } else {
        inner
    }
}

/// Produce the byte sequence for a bare key, factoring in Ctrl and Shift but
/// not Alt (Alt wraps the result with an ESC prefix — see `key_to_bytes`).
fn bare_key_to_bytes(bare: &BareKey, ctrl: bool, shift: bool) -> Vec<u8> {
    match bare {
        // ── Printable characters ─────────────────────────────────
        BareKey::Char(c) => {
            if ctrl {
                // Ctrl+letter → control byte 0x01–0x1A (Ctrl+A = 1, …, Ctrl+Z = 26).
                // Also handle a handful of common Ctrl+symbol combos.
                ctrl_char_bytes(*c)
            } else {
                // Plain or Shift-modified char — encode as UTF-8.
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }

        // ── Enter ────────────────────────────────────────────────
        BareKey::Enter => {
            if ctrl {
                vec![0x0a] // Ctrl+Enter → LF (some apps distinguish this)
            } else {
                vec![0x0d] // CR
            }
        }

        // ── Tab ──────────────────────────────────────────────────
        BareKey::Tab => {
            if ctrl {
                // Ctrl+Tab — no universal standard; send as-is (apps vary).
                vec![0x09]
            } else if shift {
                vec![0x1b, b'[', b'Z'] // ESC [ Z  (Back-Tab / Shift+Tab)
            } else {
                vec![0x09] // HT
            }
        }

        // ── Backspace ────────────────────────────────────────────
        BareKey::Backspace => {
            if ctrl {
                vec![0x08] // Ctrl+Backspace → BS
            } else {
                vec![0x7f] // DEL (modern default for Backspace)
            }
        }

        // ── Escape ───────────────────────────────────────────────
        BareKey::Esc => vec![0x1b],

        // ── Delete (forward-delete) ──────────────────────────────
        BareKey::Delete => {
            if ctrl {
                vec![0x1b, b'[', b'3', b';', b'5', b'~'] // ESC [ 3 ; 5 ~
            } else if shift {
                vec![0x1b, b'[', b'3', b';', b'2', b'~'] // ESC [ 3 ; 2 ~
            } else {
                vec![0x1b, b'[', b'3', b'~'] // ESC [ 3 ~
            }
        }

        // ── Insert ───────────────────────────────────────────────
        BareKey::Insert => {
            if shift {
                vec![0x1b, b'[', b'2', b';', b'2', b'~']
            } else {
                vec![0x1b, b'[', b'2', b'~']
            }
        }

        // ── Arrow keys ───────────────────────────────────────────
        // With Ctrl or Shift the modifier is encoded as a parameter:
        //   ESC [ <letter>          — plain
        //   ESC [ 1 ; 2 <letter>   — Shift
        //   ESC [ 1 ; 5 <letter>   — Ctrl
        //   ESC [ 1 ; 6 <letter>   — Ctrl+Shift
        BareKey::Up => arrow_seq(b'A', ctrl, shift),
        BareKey::Down => arrow_seq(b'B', ctrl, shift),
        BareKey::Right => arrow_seq(b'C', ctrl, shift),
        BareKey::Left => arrow_seq(b'D', ctrl, shift),

        // ── Home / End ───────────────────────────────────────────
        BareKey::Home => {
            if ctrl || shift {
                let m = modifier_param(ctrl, shift);
                vec![0x1b, b'[', b'1', b';', m, b'H']
            } else {
                vec![0x1b, b'[', b'H']
            }
        }
        BareKey::End => {
            if ctrl || shift {
                let m = modifier_param(ctrl, shift);
                vec![0x1b, b'[', b'1', b';', m, b'F']
            } else {
                vec![0x1b, b'[', b'F']
            }
        }

        // ── Page Up / Page Down ──────────────────────────────────
        BareKey::PageUp => {
            if ctrl || shift {
                let m = modifier_param(ctrl, shift);
                vec![0x1b, b'[', b'5', b';', m, b'~']
            } else {
                vec![0x1b, b'[', b'5', b'~']
            }
        }
        BareKey::PageDown => {
            if ctrl || shift {
                let m = modifier_param(ctrl, shift);
                vec![0x1b, b'[', b'6', b';', m, b'~']
            } else {
                vec![0x1b, b'[', b'6', b'~']
            }
        }

        // ── Function keys F1–F12 ─────────────────────────────────
        // F1–F4 use SS3 sequences; F5–F12 use CSI ~ sequences.
        BareKey::F(n) => fkey_bytes(*n, ctrl, shift),

        // ── System keys (no meaningful stdin byte sequence) ──────
        // CapsLock, NumLock, etc. do not produce stdin bytes in normal
        // terminal usage.  Send nothing — the application won't miss them.
        BareKey::CapsLock
        | BareKey::ScrollLock
        | BareKey::NumLock
        | BareKey::PrintScreen
        | BareKey::Pause
        | BareKey::Menu => vec![],
    }
}

/// Build the CSI sequence for an arrow key, incorporating modifier state.
///
/// Plain:        ESC [ <final>
/// With mods:    ESC [ 1 ; <mod> <final>
fn arrow_seq(final_byte: u8, ctrl: bool, shift: bool) -> Vec<u8> {
    if ctrl || shift {
        let m = modifier_param(ctrl, shift);
        vec![0x1b, b'[', b'1', b';', m, final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// Compute the XTerm modifier parameter byte for Ctrl/Shift combinations.
///
/// | Shift | Ctrl | param |
/// |-------|------|-------|
/// |   ✓   |      |   2   |
/// |       |  ✓   |   5   |
/// |   ✓   |  ✓   |   6   |
fn modifier_param(ctrl: bool, shift: bool) -> u8 {
    match (ctrl, shift) {
        (false, true) => b'2',
        (true, false) => b'5',
        (true, true) => b'6',
        (false, false) => b'1', // shouldn't be called without a modifier
    }
}

/// Encode Ctrl+<char> as a control byte.
///
/// Standard mapping: Ctrl+A = 0x01, …, Ctrl+Z = 0x1A.
/// A few non-letter chars that commonly produce control bytes are also handled.
fn ctrl_char_bytes(c: char) -> Vec<u8> {
    let lower = c.to_ascii_lowercase();
    let byte = match lower {
        'a'..='z' => (lower as u8) - b'a' + 1, // 0x01–0x1A
        ' ' => 0x00,                           // Ctrl+Space → NUL
        '[' => 0x1b,                           // Ctrl+[ → ESC
        '\\' => 0x1c,                          // Ctrl+\ → FS
        ']' => 0x1d,                           // Ctrl+] → GS
        '^' => 0x1e,                           // Ctrl+^ → RS
        '_' => 0x1f,                           // Ctrl+_ → US
        _ => {
            // Unknown Ctrl+char — encode the raw char as UTF-8 as a best-effort
            // fallback; the application may not interpret it, but at least
            // input is not silently dropped.
            let mut buf = [0u8; 4];
            return c.encode_utf8(&mut buf).as_bytes().to_vec();
        }
    };
    vec![byte]
}

/// Encode F1–F12, with optional Ctrl/Shift modifiers.
fn fkey_bytes(n: u8, ctrl: bool, shift: bool) -> Vec<u8> {
    if ctrl || shift {
        // XTerm extended: ESC [ <vt_code> ; <mod> ~
        // (F1–F4 get vt codes 11–14 in this form)
        let vt_code: &[u8] = match n {
            1 => b"11",
            2 => b"12",
            3 => b"13",
            4 => b"14",
            5 => b"15",
            6 => b"17",
            7 => b"18",
            8 => b"19",
            9 => b"20",
            10 => b"21",
            11 => b"23",
            12 => b"24",
            _ => return vec![],
        };
        let m = modifier_param(ctrl, shift);
        let mut seq = vec![0x1b, b'['];
        seq.extend_from_slice(vt_code);
        seq.extend_from_slice(&[b';', m, b'~']);
        seq
    } else {
        // Plain (no modifier): F1–F4 use SS3, F5–F12 use CSI ~.
        match n {
            1 => vec![0x1b, b'O', b'P'],
            2 => vec![0x1b, b'O', b'Q'],
            3 => vec![0x1b, b'O', b'R'],
            4 => vec![0x1b, b'O', b'S'],
            5 => vec![0x1b, b'[', b'1', b'5', b'~'],
            6 => vec![0x1b, b'[', b'1', b'7', b'~'],
            7 => vec![0x1b, b'[', b'1', b'8', b'~'],
            8 => vec![0x1b, b'[', b'1', b'9', b'~'],
            9 => vec![0x1b, b'[', b'2', b'0', b'~'],
            10 => vec![0x1b, b'[', b'2', b'1', b'~'],
            11 => vec![0x1b, b'[', b'2', b'3', b'~'],
            12 => vec![0x1b, b'[', b'2', b'4', b'~'],
            _ => vec![],
        }
    }
}

fn bare_key_to_shortcut_key(bare: &BareKey) -> ShortcutKey {
    match bare {
        BareKey::Char(c) => ShortcutKey::Char(*c),
        BareKey::Enter => ShortcutKey::Enter,
        BareKey::Tab => ShortcutKey::Tab,
        BareKey::Backspace => ShortcutKey::Backspace,
        BareKey::Delete => ShortcutKey::Delete,
        BareKey::Esc => ShortcutKey::Esc,
        BareKey::Insert => ShortcutKey::Insert,
        BareKey::Left => ShortcutKey::Left,
        BareKey::Right => ShortcutKey::Right,
        BareKey::Up => ShortcutKey::Up,
        BareKey::Down => ShortcutKey::Down,
        BareKey::Home => ShortcutKey::Home,
        BareKey::End => ShortcutKey::End,
        BareKey::PageUp => ShortcutKey::PageUp,
        BareKey::PageDown => ShortcutKey::PageDown,
        BareKey::F(n) => ShortcutKey::F(*n),
        // For any other key used in a chord, represent as a debug string via
        // Char with a placeholder — this is an edge case (e.g. Ctrl+CapsLock).
        BareKey::CapsLock => ShortcutKey::Char('⇪'),
        BareKey::ScrollLock => ShortcutKey::Char('⤓'),
        BareKey::NumLock => ShortcutKey::Char('⇭'),
        BareKey::PrintScreen => ShortcutKey::Char('⎙'),
        BareKey::Pause => ShortcutKey::Char('⏸'),
        BareKey::Menu => ShortcutKey::Char('≡'),
    }
}
