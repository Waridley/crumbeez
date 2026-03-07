# Development Plan & Implementation Guide

This document is the hands-on guide to implementing **crumbeez**, the Zellij session-tracker plugin. It corresponds primarily to Phases 1–3 (MVP) of the roadmap described in `DESIGN.md`.

## Prerequisites

Before starting implementation:

1. **Install Zellij** (0.41+)
   ```bash
   # See https://github.com/zellij-org/zellij#how-do-i-install-it
   # Or use cargo:
   cargo install zellij
   ```

2. **Install Rust toolchain**
   ```bash
   # See https://rustup.rs/
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. **Add WASM target**
   ```bash
   rustup target add wasm32-wasip1
   ```

4. **Familiarize yourself with Zellij**
   - Use it for a day or two
   - Understand panes, tabs, layouts
   - Try some existing plugins

## Step 1: Scaffold the Plugin

Use Zellij's plugin scaffolding tool:

```bash
# Inside a Zellij session:
zellij plugin -f -- https://github.com/zellij-org/create-rust-plugin/releases/latest/download/create-rust-plugin.wasm
```

When prompted:
- **Plugin name:** `crumbeez`
- **Project directory:** `/data/kevin/Projects/crumbeez`

This will create:
- `Cargo.toml` - Rust project configuration
- `src/main.rs` - Plugin skeleton
- `plugin.yaml` - Plugin metadata
- Development environment with auto-reload

## Step 2: Understand the Generated Code

The scaffold creates a basic plugin with:

```rust
use zellij_tile::prelude::*;

#[derive(Default)]
struct State {
    // Your plugin state
}

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        // Called once on plugin load
        request_permission(&[...]);
        subscribe(&[...]);
    }

    fn update(&mut self, event: Event) -> bool {
        // Called when subscribed events occur
        // Return true to trigger render()
        false
    }

	    fn render(&mut self, rows: usize, cols: usize) {
	        // Called when plugin should draw UI
	        println!("Hello from crumbeez!");
	    }
}

register_plugin!(State);
```

## Step 3: Set Up Development Workflow

The scaffolding tool creates a development environment with:
- **Pane 1:** Your editor (opens `src/main.rs`)
- **Pane 2:** Development plugin (auto-reloads on changes)
- **Pane 3:** Your plugin running

To rebuild and reload: Press `Ctrl+Shift+r`

## Step 4: Implement Basic Event Collection

Start with minimal functionality:

```rust
use zellij_tile::prelude::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct State {
    pane_info: BTreeMap<u32, PaneInfo>,
    events_log: Vec<String>,
}

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
        ]);
        
        subscribe(&[
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::FileSystemUpdate,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PaneUpdate(pane_manifest) => {
                // Log pane changes
                self.events_log.push(format!("Pane update: {} panes", pane_manifest.panes.len()));
                true
            }
            Event::FileSystemUpdate(paths) => {
                // Log file changes
                for path in paths {
                    self.events_log.push(format!("File updated: {}", path.display()));
                }
                true
            }
            _ => false
        }
    }

	    fn render(&mut self, rows: usize, cols: usize) {
	        // Display recent events
	        println!("crumbeez - Recent Events:");
        for (i, event) in self.events_log.iter().rev().take(rows - 2).enumerate() {
            println!("{}: {}", i, event);
        }
    }
}

register_plugin!(State);
```

Build and test:
```bash
cargo build
# Press Ctrl+Shift+r in the dev environment
```

## Step 5: Add Summarization Orchestration (Tasks + Safety Timer)

Use timers as a safety net rather than as the primary driver of summaries. In the real plugin, you’ll trigger summarization when logical tasks complete (commands, test runs, commits); the timer just ensures we still checkpoint progress if work runs for a long time without a clear boundary:

```rust
impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        // ... existing code ...
        
        subscribe(&[
            // ... existing subscriptions ...
            EventType::Timer,
        ]);
        
        // Safety timer: ensure we checkpoint at least every 10 minutes if needed
        set_timeout(600.0);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Timer(_) => {
                // Trigger a checkpoint summarization for any unsummarized activity
                // (you’ll also call summarize_recent_events() when you detect logical
                // task boundaries like a command or test run completing).
                self.summarize_recent_events();
                
                // Reset timer
                set_timeout(600.0);
                true
            }
            // ... other events ...
        }
    }
}
```


## Step 6: Pane Content Tracking

The plugin needs to read pane contents to capture command output, build errors, file views, etc. This is critical for generating useful summaries but must be done carefully to avoid spamming the LLM context.

### Architecture

```
crates/zellij-plugin/src/
├── pane_content/
│   ├── mod.rs           # Public interface
│   ├── tracker.rs       # Per-pane state tracking
│   ├── processor.rs     # Content diffing/compression
│   ├── strategies/
│   │   ├── mod.rs       # Strategy trait + dispatcher
│   │   ├── stdio.rs     # Default: line-based, dedup
│   │   ├── tui.rs       # Cursor movement detection → snapshot
│   │   └── progress.rs  # \r detection → condense progress
│   └── detectors/
│       ├── mod.rs       # Program detection trait
│       └── control_chars.rs  # ANSI/control char analysis
```

### Core Types

```rust
// Add to KeystrokeEvent enum in crumbeez-lib
pub enum KeystrokeEvent {
    // ... existing variants ...
    PaneOutput(PaneOutputEvent),
}

pub struct PaneOutputEvent {
    pub pane_id: u32,
    pub pane_title: String,
    pub command: Option<String>,
    pub output_type: OutputType,
    pub content: String,           // Processed/compressed
    pub raw_lines: usize,          // For transparency ("50 lines compressed to 3")
    pub trigger: OutputTrigger,    // Why we emitted this
}

pub enum OutputType {
    Full,           // Initial capture or TUI snapshot
    Diff,           // New lines added
    ProgressFinal,  // Final state of a progress indicator
    Truncated,      // Large output, truncated with summary
}

pub enum OutputTrigger {
    PaneSwitch,     // User switched away from this pane
    CommandExit,    // Detected shell prompt / exit
    MaxAccumulated, // Buffer full, forced emit
}
```

### Content Processing Pipeline

```
PaneRenderReport (HashMap<PaneId, PaneContents>)
       ↓
  Per-pane Tracker
  (hash compare → skip if unchanged)
       ↓
  Control Char Analysis
  (detect TUI vs stdio vs progress)
       ↓
  Strategy Selection
  (future: program-specific overrides)
       ↓
  Diff/Compress
  (dedupe repeated lines, strip ANSI, truncate)
       ↓
  Accumulate in Buffer
       ↓
  Emit on Boundary
  (pane switch, command exit, buffer full)
```

### Key Implementation Details

1. **Watch All Panes**: Subscribe to `PaneRenderReport` for ALL terminal panes, not just focused. Background tasks matter.

2. **Diffing Strategy**:
   - Store last viewport as `Vec<String>` per pane
   - For stdio: compute new lines = current - last (by line hash)
   - For TUI: if cursor positioning codes detected, store full snapshot

3. **Progress Bar Compression**:
   - Detect `\r` (carriage return without newline) → in-place update
   - Track "progress lines" separately
   - Emit only the final state when newline appears or content changes

4. **Repeated Line Deduplication**:
   ```
   Instead of: "Building...\n" (50 times)
   Emit: "[50×] Building..."
   ```

5. **ANSI Code Analysis** (no stripping):
   - Preserve ANSI codes in logged content for LLM context
   - Use control codes to detect program type:
     - Cursor positioning (`\x1b[;H`) → TUI
     - Clear screen (`\x1b[2J`) → TUI
     - Color codes only → stdio with colors

6. **Semantic Boundary Detection**:
   - Emit accumulated output when:
     - User switches panes
     - Shell prompt detected (regex match on last line)
     - Buffer exceeds threshold
   - Future: quiescence timeout (configurable)

### Future Extensibility

The `strategies/` module uses a trait-based approach for program-specific handlers:

```rust
pub trait ContentStrategy: Send + Sync {
    fn process(&self, input: &str, state: &mut StrategyState) -> ProcessResult;
    fn should_emit(&self, state: &StrategyState) -> bool;
    fn detect(&self, content: &str) -> DetectionConfidence;
}
```

This allows adding handlers for:
- `cargo` / `npm` build output
- `git` commands
- `man` pages
- Test runners
- etc.

### Implementation Order

1. `pane_content/mod.rs` + `tracker.rs` - Basic pane tracking with hash-based deduplication
2. `pane_content/processor.rs` - Diffing logic
3. `pane_content/strategies/stdio.rs` - Default strategy with line deduplication
4. `pane_content/strategies/tui.rs` - TUI detection and snapshot mode
5. `pane_content/strategies/progress.rs` - Progress bar compression
6. Integration with `KeystrokeEvent` and event logging
7. Future: Program-specific strategies, quiescence detection, configuration


## Step 7: Integrate LLM API

Add web request capability:

```rust
impl State {
    fn summarize_recent_events(&mut self) {
        // Get unsummarized events from DB
        let events = self.event_store.get_unsummarized_events();
        
        // Build prompt
        let prompt = format!(
            "Summarize these development activities:\n{}",
            events.join("\n")
        );
        
        // Call LLM API
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": prompt}]
        }).to_string();
        
        let headers = BTreeMap::from([
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Authorization".to_string(), format!("Bearer {}", self.api_key)),
        ]);
        
        let context = BTreeMap::from([
            ("request_type".to_string(), "summarize".to_string()),
        ]);
        
        web_request(
            "https://api.openai.com/v1/chat/completions",
            HttpVerb::Post,
            headers,
            body.as_bytes().to_vec(),
            context,
        );
    }
}
```

Handle the response:
```rust
Event::WebRequestResult(status, headers, body, context) => {
    if context.get("request_type") == Some(&"summarize".to_string()) {
        let response: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let summary = response["choices"][0]["message"]["content"].as_str().unwrap();
        
        // Store summary
        self.event_store.store_summary(summary);
        
        // Mark events as summarized
        self.event_store.mark_events_summarized();
    }
    true
}
```

## Step 8: Build Release Version

When ready to use:

```bash
cargo build --release
```

The plugin will be at: `target/wasm32-wasip1/release/crumbeez.wasm`

Load it in Zellij:
```bash
zellij plugin -- file:///data/kevin/Projects/crumbeez/target/wasm32-wasip1/release/crumbeez.wasm
```

Or add to your Zellij config (`~/.config/zellij/config.kdl`):
```kdl
plugins {
	crumbeez {
	    path "file:///data/kevin/Projects/crumbeez/target/wasm32-wasip1/release/crumbeez.wasm"
	}
}
```

## Resources

- **Zellij Plugin Tutorial:** https://zellij.dev/tutorials/developing-a-rust-plugin/
- **Plugin API Docs:** https://zellij.dev/documentation/plugins
- **Rust API Reference:** https://docs.rs/zellij-tile/latest/zellij_tile/
- **Example Plugins:** https://github.com/zellij-org/zellij/tree/main/default-plugins
- **Community:** https://discord.gg/CrUAFH3 (Zellij Discord)

## Tips

1. **Use eprintln! for debugging** - Output goes to Zellij log file
2. **Check log location:** `zellij setup --check`
3. **Tail logs:** `tail -f /tmp/zellij-*/zellij-log/zellij.log`
4. **Start simple** - Get basic event logging working first
5. **Iterate quickly** - Use the dev environment's auto-reload
6. **Test edge cases** - What happens when panes close? Tabs switch?

## Next Steps After MVP

Once you have basic functionality, these steps roughly correspond to Phases 4–5 of the roadmap in `DESIGN.md`:

1. Add program-specific handlers (detect editors, test runners, etc.)
2. Improve event correlation (link file edits to test runs)
3. Add query interface (search past summaries)
4. Create better UI (dedicated summary pane)
5. Add configuration options
6. Publish to awesome-zellij

