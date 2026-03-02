mod event_log;
mod summary;

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use event_log::{EventLog, EventLogError, LogEntry, Summary};
pub use summary::{DisplayNode, SummaryId, SummaryNode};

// ── Directory layout constants ───────────────────────────────────

/// Name of the crumbeez data directory created at each project root.
pub const CRUMBEEZ_DIR_NAME: &str = ".crumbeez";

/// Subdirectory for human-readable summary logs (Markdown), stored in-repo.
pub const SUMMARIES_SUBDIR: &str = "summaries";

/// Event log file name.
///
/// In the WASM plugin this is written to `/data/events.bin`, which the WASI
/// runtime maps to the plugin's own per-session cache directory.
pub const EVENT_LOG_FILE: &str = "events.bin";

// ── Directory layout helpers ─────────────────────────────────────

/// Returns the `.crumbeez` directory path for a given project root.
pub fn crumbeez_dir(root: &Path) -> PathBuf {
    root.join(CRUMBEEZ_DIR_NAME)
}

/// Returns the summaries subdirectory path for a given project root.
pub fn summaries_dir(root: &Path) -> PathBuf {
    crumbeez_dir(root).join(SUMMARIES_SUBDIR)
}

/// Returns all in-repo directories that must exist for a given project root.
pub fn required_project_dirs(root: &Path) -> Vec<PathBuf> {
    vec![summaries_dir(root)]
}

// ── Discovery phase ──────────────────────────────────────────────

/// Async state machine phases for root discovery.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum DiscoveryPhase {
    /// Waiting for RunCommands permission to be granted.
    #[default]
    AwaitingPermissions,
    /// Fired `git rev-parse --show-toplevel`, waiting for result.
    FindingGitRoot,
    /// Fired `git rev-parse --show-superproject-working-tree`, waiting for result.
    FindingSuperproject,
    /// Fired `mkdir -p` commands, waiting for them to complete.
    CreatingDirs {
        pending: usize,
        /// In-repo `.crumbeez` directories (one per project root).
        project_dirs: Vec<PathBuf>,
    },
    /// All directories have been created and are ready.
    Ready {
        /// In-repo `.crumbeez` directories (for summaries, etc.).
        project_dirs: Vec<PathBuf>,
    },
    /// Discovery failed with an error message.
    Failed(String),
}

impl fmt::Display for DiscoveryPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AwaitingPermissions => write!(f, "⏳ Awaiting permissions..."),
            Self::FindingGitRoot => write!(f, "🔍 Finding git root..."),
            Self::FindingSuperproject => write!(f, "🔍 Checking for parent repo..."),
            Self::CreatingDirs { pending, .. } => {
                write!(f, "📁 Creating directories ({pending} remaining)...")
            }
            Self::Ready { project_dirs } => {
                let dirs: Vec<_> = project_dirs.iter().map(|d| d.to_string_lossy()).collect();
                write!(f, "✅ Ready — projects: {}", dirs.join(", "))
            }
            Self::Failed(msg) => write!(f, "❌ Failed: {msg}"),
        }
    }
}

// ── Keystroke activity ───────────────────────────────────────────

/// Maximum number of recent keystroke events kept in the activity log.
pub const KEYSTROKE_LOG_CAPACITY: usize = 200;

/// A semantic classification of a single keystroke or chord.
///
/// The goal is to preserve enough fidelity for an LLM to understand what the
/// user was doing without forwarding every raw keycode verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum KeystrokeEvent {
    /// One or more printable characters typed with no non-Shift modifiers.
    /// Consecutive text keystrokes are coalesced into a single event so the
    /// log stays compact.  The string may contain unicode.
    TextTyped(String),

    /// A keyboard shortcut: a key chord that includes Ctrl, Alt, or Super.
    /// Examples: Ctrl+S, Ctrl+Shift+Z, Alt+F4, Super+L.
    Shortcut(ShortcutEvent),

    /// A navigation keystroke: arrow keys, Home, End, PageUp, PageDown.
    /// Consecutive navigation events of the same kind are coalesced with a
    /// repeat count so rapid cursor movement doesn't flood the log.
    Navigation(NavigationEvent),

    /// An editing control key: Enter, Tab, Backspace, Delete, Insert.
    EditControl(EditControlEvent),

    /// An escape / cancel keystroke (Esc).
    Escape,

    /// A function key pressed without any modifier (F1–F12).
    FunctionKey(u8),

    /// A system-level key: CapsLock, ScrollLock, NumLock, PrintScreen, Pause,
    /// Menu.  These are uncommon but worth noting.
    SystemKey(SystemKeyEvent),

    /// The user switched to a different pane (or the session focus changed on
    /// startup).  This is a context boundary: subsequent keystrokes are being
    /// sent to a different program.
    PaneFocused(PaneFocusedEvent),

    /// Output captured from a terminal pane.  Emitted when a semantic boundary
    /// is detected (pane switch, shell prompt, buffer full, etc.).
    /// Only produced when the `pane-content-tracking` feature is active.
    PaneOutput(PaneOutputEvent),
}

impl fmt::Display for KeystrokeEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TextTyped(s) => write!(f, "typed {:?}", s),
            Self::Shortcut(s) => write!(f, "shortcut {}", s),
            Self::Navigation(n) => write!(f, "nav {}", n),
            Self::EditControl(e) => write!(f, "edit-ctrl {}", e),
            Self::Escape => write!(f, "Esc"),
            Self::FunctionKey(n) => write!(f, "F{}", n),
            Self::SystemKey(k) => write!(f, "sys {}", k),
            Self::PaneFocused(p) => write!(f, "focus → {}", p),
            Self::PaneOutput(o) => write!(f, "pane-output {}", o),
        }
    }
}

// ── ShortcutEvent ────────────────────────────────────────────────

/// A keyboard shortcut — a chord involving Ctrl, Alt, or Super.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortcutEvent {
    /// The base key (printable char, function key number, named key, etc.).
    pub key: ShortcutKey,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
}

impl fmt::Display for ShortcutEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl+")?;
        }
        if self.alt {
            write!(f, "Alt+")?;
        }
        if self.shift {
            write!(f, "Shift+")?;
        }
        if self.super_key {
            write!(f, "Super+")?;
        }
        write!(f, "{}", self.key)
    }
}

/// The base key of a shortcut chord.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ShortcutKey {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Esc,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

impl fmt::Display for ShortcutKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Char(c) => write!(f, "{}", c),
            Self::Enter => write!(f, "Enter"),
            Self::Tab => write!(f, "Tab"),
            Self::Backspace => write!(f, "Backspace"),
            Self::Delete => write!(f, "Delete"),
            Self::Esc => write!(f, "Esc"),
            Self::Insert => write!(f, "Insert"),
            Self::Left => write!(f, "←"),
            Self::Right => write!(f, "→"),
            Self::Up => write!(f, "↑"),
            Self::Down => write!(f, "↓"),
            Self::Home => write!(f, "Home"),
            Self::End => write!(f, "End"),
            Self::PageUp => write!(f, "PgUp"),
            Self::PageDown => write!(f, "PgDn"),
            Self::F(n) => write!(f, "F{}", n),
        }
    }
}

// ── NavigationEvent ──────────────────────────────────────────────

/// A navigation keystroke, with repetition count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NavigationEvent {
    pub direction: NavDirection,
    /// How many consecutive times this key was pressed.
    pub count: usize,
    /// Whether a modifier key (typically Shift or Ctrl) was held.
    pub with_shift: bool,
    pub with_ctrl: bool,
}

impl fmt::Display for NavigationEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.with_ctrl {
            write!(f, "Ctrl+")?;
        }
        if self.with_shift {
            write!(f, "Shift+")?;
        }
        write!(f, "{}", self.direction)?;
        if self.count > 1 {
            write!(f, " ×{}", self.count)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NavDirection {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

impl fmt::Display for NavDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Left => write!(f, "←"),
            Self::Right => write!(f, "→"),
            Self::Up => write!(f, "↑"),
            Self::Down => write!(f, "↓"),
            Self::Home => write!(f, "Home"),
            Self::End => write!(f, "End"),
            Self::PageUp => write!(f, "PgUp"),
            Self::PageDown => write!(f, "PgDn"),
        }
    }
}

// ── EditControlEvent ─────────────────────────────────────────────

/// An editing control keystroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EditControlEvent {
    Enter,
    Tab,
    /// Backspace, with repetition count for consecutive presses.
    Backspace {
        count: usize,
    },
    /// Delete (forward-delete), with repetition count.
    Delete {
        count: usize,
    },
    Insert,
}

impl fmt::Display for EditControlEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enter => write!(f, "Enter"),
            Self::Tab => write!(f, "Tab"),
            Self::Backspace { count } if *count == 1 => write!(f, "Backspace"),
            Self::Backspace { count } => write!(f, "Backspace ×{}", count),
            Self::Delete { count } if *count == 1 => write!(f, "Delete"),
            Self::Delete { count } => write!(f, "Delete ×{}", count),
            Self::Insert => write!(f, "Insert"),
        }
    }
}

// ── SystemKeyEvent ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SystemKeyEvent {
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
}

impl fmt::Display for SystemKeyEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapsLock => write!(f, "CapsLock"),
            Self::ScrollLock => write!(f, "ScrollLock"),
            Self::NumLock => write!(f, "NumLock"),
            Self::PrintScreen => write!(f, "PrintScreen"),
            Self::Pause => write!(f, "Pause"),
            Self::Menu => write!(f, "Menu"),
        }
    }
}

// ── PaneFocusedEvent ─────────────────────────────────────────────

/// Describes the pane that just received keyboard focus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneFocusedEvent {
    /// The tab name, if known and non-empty.
    pub tab_name: Option<String>,
    /// The pane title as shown in the Zellij UI (the window title set by the
    /// running program, e.g. "nvim README.md" or "bash").
    pub pane_title: String,
    /// The raw command string for terminal panes (e.g. "/bin/bash"), if
    /// available.  `None` for plugin panes.
    pub command: Option<String>,
    /// `true` when this is a plugin pane rather than a terminal pane.
    pub is_plugin: bool,
}

impl fmt::Display for PaneFocusedEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Build the bracket label: tab name and/or shell command basename.
        // e.g. "[tab 1 (nu)] OC | Project Purpose Review"
        let cmd_basename = self
            .command
            .as_deref()
            .map(|cmd| cmd.rsplit('/').next().unwrap_or(cmd));

        match (&self.tab_name, cmd_basename) {
            (Some(tab), Some(cmd)) => write!(f, "[{} ({})] ", tab, cmd)?,
            (Some(tab), None) => write!(f, "[{}] ", tab)?,
            (None, Some(cmd)) => write!(f, "[({})] ", cmd)?,
            (None, None) => {}
        }

        write!(f, "{}", self.pane_title)
    }
}

// ── PaneOutputEvent ──────────────────────────────────────────────

/// Captured output from a terminal pane, emitted at a semantic boundary.
///
/// The `content` field holds the processed/compressed text ready for logging.
/// `raw_lines` records how many raw lines were consumed to produce it, giving
/// the LLM context like "[50 lines → 3]".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneOutputEvent {
    /// The numeric pane id (matches zellij's `PaneId::Terminal`).
    pub pane_id: u32,
    /// The pane title as shown in the Zellij UI at the time of emission.
    pub pane_title: String,
    /// The foreground command reported by the pane, if any (e.g. `"cargo"`).
    pub command: Option<String>,
    /// Classification of the captured output.
    pub output_type: OutputType,
    /// Processed content (preserves ANSI for LLM context, deduped, compressed).
    pub content: String,
    /// Number of raw viewport lines that were consumed to produce `content`.
    pub raw_lines: usize,
    /// Why this event was emitted now.
    pub trigger: OutputTrigger,
}

impl fmt::Display for PaneOutputEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cmd = self.command.as_deref().unwrap_or("?");
        write!(
            f,
            "[pane {}] {} ({:?}, {} raw lines, {:?}): {}",
            self.pane_id,
            cmd,
            self.output_type,
            self.raw_lines,
            self.trigger,
            // Truncate very long content for Display
            if self.content.len() > 80 {
                format!("{}…", &self.content[..80])
            } else {
                self.content.clone()
            }
        )
    }
}

/// Classification of captured pane output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OutputType {
    /// Complete viewport snapshot (first capture or after TUI mode).
    Full,
    /// Only the lines added since the last emission (stdio mode).
    Diff,
    /// Final state of a progress indicator (e.g. download/build bar).
    ProgressFinal,
    /// Output was too large; truncated with a "N lines omitted" note.
    Truncated,
}

/// Why a `PaneOutputEvent` was emitted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OutputTrigger {
    /// User switched focus away from this pane.
    PaneSwitch,
    /// A shell prompt was detected, indicating a command completed.
    CommandExit,
    /// The accumulation buffer hit the maximum-lines threshold.
    MaxAccumulated,
}

// ── KeystrokeActivity ────────────────────────────────────────────

/// Accumulates and classifies keystroke events, applying editing operations
/// (backspace, delete, cursor movement) directly to the in-progress
/// [`KeystrokeEvent::TextTyped`] buffer so the log reflects what the user
/// actually typed — net of corrections — rather than a raw key stream.
///
/// ### Editing model
///
/// A `TextTyped` entry at the tail of the log is treated as a live buffer
/// while cursor-movement and edit-control keys keep arriving.  A byte-level
/// cursor tracks the insertion point inside that buffer.  Once a *sealing*
/// event arrives (Enter, Esc, Tab, any shortcut, Up/Down/PageUp/PageDown, or
/// any non-editing event) the buffer is frozen and subsequent keystrokes start
/// a new entry.
///
/// Keys handled within the live buffer:
///
/// | Key | Effect |
/// |-----|--------|
/// | Printable char | Insert at cursor, advance cursor |
/// | Backspace | Delete char *before* cursor (if any) |
/// | Delete | Delete char *at* cursor (if any) |
/// | ← / → | Move cursor one Unicode scalar left / right |
/// | Ctrl+← / Ctrl+→ | Move cursor one word left / right |
/// | Home | Move cursor to start of buffer |
/// | End | Move cursor to end of buffer |
/// | Up / Down / PgUp / PgDn | Seal the buffer (left the line context) |
///
/// If backspace/delete empties the buffer the `TextTyped` entry is removed
/// rather than left as an empty string.  An empty buffer is never stored.
///
/// This type lives in `crumbeez-lib` (no Zellij dependency) so it can be
/// unit-tested on native targets.
#[derive(Debug, Default)]
pub struct KeystrokeActivity {
    /// Bounded ring-buffer of completed semantic events.
    events: VecDeque<KeystrokeEvent>,
    /// Byte offset of the cursor inside the tail `TextTyped` buffer, if one
    /// is currently live.  `None` when the tail is not a `TextTyped` entry.
    cursor: Option<usize>,
}

impl KeystrokeActivity {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return all logged events (oldest first).
    pub fn events(&self) -> &VecDeque<KeystrokeEvent> {
        &self.events
    }

    /// Incorporate a new semantic event into the activity log.
    ///
    /// Editing keys (Backspace, Delete, cursor movement) are applied
    /// retroactively to the tail `TextTyped` buffer rather than appended as
    /// separate entries.  Everything else either continues the live buffer or
    /// seals it and is appended as a new entry.
    pub fn push_event(&mut self, event: KeystrokeEvent) {
        match &event {
            // ── Text: insert into live buffer ────────────────────
            KeystrokeEvent::TextTyped(incoming) => {
                if let Some(cursor) = self.cursor {
                    // There is already a live TextTyped buffer — insert there.
                    if let Some(KeystrokeEvent::TextTyped(ref mut buf)) = self.events.back_mut() {
                        let insertion = incoming.as_str();
                        buf.insert_str(cursor, insertion);
                        self.cursor = Some(cursor + insertion.len());
                        return;
                    }
                }
                // No live buffer — push a new one and set cursor at its end.
                let len = incoming.len();
                self.append(event);
                self.cursor = Some(len);
            }

            // ── Backspace: delete char before cursor ─────────────
            KeystrokeEvent::EditControl(EditControlEvent::Backspace { .. }) => {
                if let Some(cursor) = self.cursor {
                    if cursor > 0 {
                        if let Some(KeystrokeEvent::TextTyped(ref mut buf)) = self.events.back_mut()
                        {
                            // Find the start of the preceding Unicode scalar.
                            let prev = prev_char_boundary(buf, cursor);
                            buf.drain(prev..cursor);
                            if buf.is_empty() {
                                self.events.pop_back();
                                self.cursor = None;
                            } else {
                                self.cursor = Some(prev);
                            }
                            return;
                        }
                    } else {
                        // Cursor at start — nothing to delete; swallow the event.
                        return;
                    }
                }
                // No live buffer — append as a plain event.
                self.coalesce_or_append(event);
            }

            // ── Delete: delete char at cursor ────────────────────
            KeystrokeEvent::EditControl(EditControlEvent::Delete { .. }) => {
                if let Some(cursor) = self.cursor {
                    if let Some(KeystrokeEvent::TextTyped(ref mut buf)) = self.events.back_mut() {
                        if cursor < buf.len() {
                            let next = next_char_boundary(buf, cursor);
                            buf.drain(cursor..next);
                            if buf.is_empty() {
                                self.events.pop_back();
                                self.cursor = None;
                            }
                            // cursor stays at same position (now points at what
                            // was the next character)
                            return;
                        } else {
                            // Cursor at end — nothing to delete; swallow.
                            return;
                        }
                    }
                }
                self.coalesce_or_append(event);
            }

            // ── Navigation: move cursor or seal ──────────────────
            KeystrokeEvent::Navigation(nav) => {
                match nav.direction {
                    // Left / Right move cursor within the live buffer.
                    NavDirection::Left | NavDirection::Right => {
                        if let Some(cursor) = self.cursor {
                            if let Some(KeystrokeEvent::TextTyped(ref buf)) = self.events.back() {
                                let new_cursor = if nav.direction == NavDirection::Left {
                                    if nav.with_ctrl {
                                        word_left(buf, cursor)
                                    } else {
                                        // Move left by nav.count characters.
                                        let mut pos = cursor;
                                        for _ in 0..nav.count {
                                            pos = prev_char_boundary(buf, pos);
                                        }
                                        pos
                                    }
                                } else {
                                    // Right
                                    if nav.with_ctrl {
                                        word_right(buf, cursor)
                                    } else {
                                        let mut pos = cursor;
                                        for _ in 0..nav.count {
                                            pos = next_char_boundary(buf, pos);
                                        }
                                        pos
                                    }
                                };
                                self.cursor = Some(new_cursor);
                                return;
                            }
                        }
                        // No live buffer — append navigation as an event.
                        self.coalesce_or_append(event);
                    }

                    // Home / End jump to buffer boundaries.
                    NavDirection::Home => {
                        if self.cursor.is_some() {
                            self.cursor = Some(0);
                            return;
                        }
                        self.coalesce_or_append(event);
                    }
                    NavDirection::End => {
                        if self.cursor.is_some() {
                            if let Some(KeystrokeEvent::TextTyped(ref buf)) = self.events.back() {
                                self.cursor = Some(buf.len());
                                return;
                            }
                        }
                        self.coalesce_or_append(event);
                    }

                    // Up / Down / PageUp / PageDown leave the current line —
                    // seal the buffer and append as a normal navigation event.
                    NavDirection::Up
                    | NavDirection::Down
                    | NavDirection::PageUp
                    | NavDirection::PageDown => {
                        self.cursor = None;
                        self.coalesce_or_append(event);
                    }
                }
            }

            // ── Sealing events ───────────────────────────────────
            // Enter, Tab, Esc, shortcuts, function keys, system keys — all
            // seal the live buffer and are appended as their own entries.
            _ => {
                self.cursor = None;
                self.coalesce_or_append(event);
            }
        }
    }

    /// Clear all logged events and reset cursor state.
    pub fn clear(&mut self) {
        self.events.clear();
        self.cursor = None;
    }

    // ── Internal helpers ─────────────────────────────────────────

    /// Append `event`, enforcing the capacity limit.
    fn append(&mut self, event: KeystrokeEvent) {
        if self.events.len() >= KEYSTROKE_LOG_CAPACITY {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// Try to coalesce `event` into the tail entry; if not possible, append.
    /// Used for events that don't touch the live text buffer (navigation runs,
    /// Backspace/Delete outside a buffer, etc.).
    fn coalesce_or_append(&mut self, event: KeystrokeEvent) {
        if let Some(last) = self.events.back_mut() {
            if try_coalesce(last, &event) {
                return;
            }
        }
        self.append(event);
    }
}

// ── Coalescing ───────────────────────────────────────────────────

/// Try to merge `new` into `last` in-place for run-length–style compaction.
/// Returns `true` if the merge happened (caller should not push separately).
fn try_coalesce(last: &mut KeystrokeEvent, new: &KeystrokeEvent) -> bool {
    match (last, new) {
        // Consecutive Backspace / Delete outside a live buffer → increment count.
        (
            KeystrokeEvent::EditControl(EditControlEvent::Backspace { count }),
            KeystrokeEvent::EditControl(EditControlEvent::Backspace { .. }),
        ) => {
            *count += 1;
            true
        }
        (
            KeystrokeEvent::EditControl(EditControlEvent::Delete { count }),
            KeystrokeEvent::EditControl(EditControlEvent::Delete { .. }),
        ) => {
            *count += 1;
            true
        }

        // Repeated navigation in the same direction with same modifiers.
        (KeystrokeEvent::Navigation(ref mut prev), KeystrokeEvent::Navigation(next))
            if prev.direction == next.direction
                && prev.with_shift == next.with_shift
                && prev.with_ctrl == next.with_ctrl =>
        {
            prev.count += next.count;
            true
        }

        _ => false,
    }
}

// ── Unicode cursor helpers ───────────────────────────────────────

/// Return the byte offset of the start of the Unicode scalar *before* `pos`.
/// Clamps to 0 if already at the start.
fn prev_char_boundary(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = pos - 1;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Return the byte offset immediately after the Unicode scalar starting at
/// `pos`.  Clamps to `s.len()` if already at the end.
fn next_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

/// Move the cursor one word to the left (Ctrl+←).
///
/// Word boundary: the last transition from a non-alphanumeric char to an
/// alphanumeric char to the left of `pos`.
fn word_left(s: &str, pos: usize) -> usize {
    let chars_before: Vec<(usize, char)> = s[..pos].char_indices().collect();
    if chars_before.is_empty() {
        return 0;
    }
    // Skip trailing non-word chars, then skip the word.
    let mut iter = chars_before.iter().rev();
    // Skip leading whitespace/punctuation
    for &(_, c) in iter.by_ref() {
        if c.is_alphanumeric() || c == '_' {
            break;
        }
    }
    // Skip the word itself
    for &(i, c) in iter {
        if !c.is_alphanumeric() && c != '_' {
            return next_char_boundary(s, i);
        }
    }
    0
}

/// Move the cursor one word to the right (Ctrl+→).
///
/// Skips the current word (if any) then any trailing whitespace/punctuation.
fn word_right(s: &str, pos: usize) -> usize {
    let chars_after: Vec<(usize, char)> =
        s[pos..].char_indices().map(|(i, c)| (pos + i, c)).collect();
    if chars_after.is_empty() {
        return s.len();
    }
    let mut iter = chars_after.iter();
    // Skip non-word chars first (in case cursor is between words)
    let mut found_word = false;
    for &(_i, c) in iter.by_ref() {
        if c.is_alphanumeric() || c == '_' {
            found_word = true;
            break;
        }
    }
    if !found_word {
        return s.len();
    }
    // Skip to end of this word
    for &(byte_i, c) in iter.by_ref() {
        if !c.is_alphanumeric() && c != '_' {
            return byte_i;
        }
    }
    s.len()
}
