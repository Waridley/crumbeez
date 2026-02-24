/// Analysis of ANSI / control-character patterns in a block of terminal text.
///
/// The detector makes a single pass over the raw bytes and classifies the
/// content into one of three modes that drive strategy selection downstream.

/// High-level classification of a pane's output mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneMode {
    /// Plain line-oriented output (may have colour codes, but no cursor
    /// repositioning).  Suitable for line-diff and deduplication.
    Stdio,
    /// Full-screen TUI: cursor-positioning sequences detected.  Requires
    /// snapshot mode — diffs are meaningless here.
    Tui,
    /// `\r` without a following `\n` detected: in-place progress updates.
    /// Treated specially to collapse spinner/bar noise into a final state.
    Progress,
}

/// Analyse `text` and return the detected [`PaneMode`].
///
/// Heuristics (checked in priority order):
/// 1. Any cursor-addressing sequence (`ESC [ <row> ; <col> H` or `ESC [ H`)
///    → `Tui`
/// 2. Any erase-display sequence (`ESC [ 2 J`) → `Tui`
/// 3. Any `\r` that is *not* immediately followed by `\n` → `Progress`
/// 4. Anything else → `Stdio`
pub fn detect_mode(text: &str) -> PaneMode {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        match bytes[i] {
            // ESC introduces a potential control sequence.
            0x1b if i + 1 < len && bytes[i + 1] == b'[' => {
                // Consume "ESC ["
                i += 2;
                // Collect parameter bytes (0x30–0x3F) and intermediate
                // bytes (0x20–0x2F).
                let seq_start = i;
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b';') {
                    i += 1;
                }
                if i < len {
                    let final_byte = bytes[i];
                    let params = &text[seq_start..i];
                    match final_byte {
                        // Cursor Position: ESC [ <row> ; <col> H  or  ESC [ H
                        b'H' => return PaneMode::Tui,
                        // Cursor Position (f alias)
                        b'f' => return PaneMode::Tui,
                        // Erase in Display: ESC [ 2 J  clears the screen
                        b'J' if params == "2" => return PaneMode::Tui,
                        // Erase entire display variant
                        b'J' if params.is_empty() => return PaneMode::Tui,
                        _ => {}
                    }
                }
            }
            // Carriage return: check whether it is a bare \r (progress) or
            // \r\n (normal line ending, ignore).
            b'\r' => {
                if i + 1 >= len || bytes[i + 1] != b'\n' {
                    return PaneMode::Progress;
                }
                i += 1; // skip the \n as well
            }
            _ => {}
        }
        i += 1;
    }

    PaneMode::Stdio
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_stdio() {
        assert_eq!(detect_mode("hello\nworld\n"), PaneMode::Stdio);
    }

    #[test]
    fn ansi_colours_are_stdio() {
        assert_eq!(detect_mode("\x1b[31mred\x1b[0m\n"), PaneMode::Stdio);
    }

    #[test]
    fn cursor_position_is_tui() {
        assert_eq!(detect_mode("\x1b[1;1H"), PaneMode::Tui);
    }

    #[test]
    fn cursor_home_is_tui() {
        assert_eq!(detect_mode("\x1b[H"), PaneMode::Tui);
    }

    #[test]
    fn erase_display_is_tui() {
        assert_eq!(detect_mode("\x1b[2J"), PaneMode::Tui);
    }

    #[test]
    fn bare_cr_is_progress() {
        assert_eq!(
            detect_mode("Building...\r[50%]\r[100%]\n"),
            PaneMode::Progress
        );
    }

    #[test]
    fn crlf_is_stdio() {
        assert_eq!(detect_mode("line1\r\nline2\r\n"), PaneMode::Stdio);
    }
}
