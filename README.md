# crumbeez

**Automatic development session tracking for terminal-based workflows**

## What is this?

A Zellij plugin that automatically tracks your development activity and generates intelligent summaries of what you've accomplished. Think of it as a session journal that writes itself.

Inspired by JetBrains' AI agent session tracking feature, but designed for terminal-based development workflows.

## Why?

When you're deep in a coding session, you're making dozens of decisions: editing files, running tests, fixing errors, committing changes. Hours later (or days later), it can be hard to remember:

- Why did I make that change?
- What was I working on before the crash?
- What did I accomplish in the last hour?
- What was I debugging when I got interrupted?

This plugin solves that by:
1. **Automatically tracking** everything you do across all terminal panes
2. **Understanding context** - not just logging, but semantic understanding
3. **Generating summaries** - LLM-powered summaries aligned with logical tasks and checkpoints in your work
4. **Never losing data** - crash-resistant storage of all events

## How it works

The plugin runs inside Zellij and:

1. **Watches all your panes** - knows when you edit files, run tests, execute builds
2. **Understands what's happening** - parses test results, build errors, git commits
3. **Correlates activities** - "edited auth.rs in pane 1, tests failed in pane 2, fixed and committed"
4. **Summarizes around logical tasks** - when it detects you finished a self-contained unit of work (like a test run, build, or commit), it sends structured events to an LLM for summarization; for very long-running tasks, it can still checkpoint progress so you don't lose context after a crash
5. **Displays summaries** - shows what you've been working on in a dedicated pane or status bar

## Key Features

- ✅ **Automatic** - No manual logging required
- ✅ **Multi-pane aware** - Understands your workflow across different panes
- ✅ **Crash resistant** - Events saved immediately to MessagePack binary log
- ✅ **Keystroke interception** - Captures and classifies all keyboard input
- ⏳ **Privacy-focused** - Local-only option with Ollama, or cloud LLMs (planned)
- ⏳ **Semantic understanding** - Intelligent interpretation of activities (planned)
- ⏳ **Task-based summaries** - LLM-powered summaries tied to logical units of work (planned)

## Status

🚧 **In Development** - Core event tracking implemented. Working on LLM summarization and human-readable summary logs.

See [DESIGN.md](./DESIGN.md) for detailed architecture and implementation plans.

## Architecture Overview

```
Zellij Panes → Event Collector → Keystroke Events
                                              ↓
                                    MessagePack Event Log
                                              ↓
                                   Summary Generation
                                              ↓
                                              LLM API
                                              ↓
                                      Summary Display
```

The plugin doesn't send raw terminal output to the LLM. Instead, it:
1. Detects what program is running (editor, test runner, compiler, etc.)
2. Extracts semantic information (which files changed, which tests failed, etc.)
3. Generates structured events
4. Sends event timelines + file diffs to the LLM when it detects logical task boundaries (like a command, test run, or commit completing), with optional time-based checkpoints for very long-running tasks

## Why Zellij Plugin (vs other approaches)?

We considered several approaches:

| Approach | Pros | Cons |
|----------|------|------|
| **PTY Wrapper** | Works everywhere, full control | Must build all infrastructure, very noisy data |
| **Shell Integration** | Lightweight, portable | Misses editor activity, no multi-pane awareness |
| **Zellij Plugin** ✅ | Session context for free, built-in UI, multi-pane awareness | Zellij-specific |

The Zellij plugin approach wins because:
- Developers who care about session tracking likely already use multiplexers
- Pane/tab structure provides valuable semantic information
- Built-in UI and event system means we can focus on intelligence, not infrastructure
- Lower barrier to entry = faster MVP

See [DESIGN.md](./DESIGN.md) for detailed comparison.

## Planned Features

### MVP (v0.1)
- [x] Keystroke tracking with semantic classification
- [x] Pane/tab focus tracking
- [x] MessagePack event storage
- [ ] Editor detection and file change tracking
- [ ] LLM summarization integration
- [ ] Human-readable summary logs
- [ ] Basic UI for viewing summaries

### Future
- [ ] Test runner output parsing (cargo test, pytest, jest, etc.)
- [ ] Build error extraction
- [ ] Git integration (commits, branches, diffs)
- [ ] Query interface ("what did I do on the auth feature?")
- [ ] Session replay
- [ ] Export to markdown
- [ ] Integration with issue trackers

## Configuration (Planned)

```kdl
// ~/.config/zellij/config.kdl
plugins {
	crumbeez {
		path "crumbeez"
        
        // LLM backend
        llm_provider "ollama"  // or "openai", "anthropic", "none"
        llm_model "llama3"
        llm_api_url "http://localhost:11434"
        
        // Summarization (task-based with optional safety checkpoints)
        max_summary_gap_minutes 15  // fail-safe: ensure some progress is logged even during long-running tasks
        
        // UI
        show_status_bar true
        summary_pane_position "bottom"
    }
}
```

## Development

Not yet ready for development. Currently in design phase.

## Contributing

Ideas and feedback welcome! Open an issue or PR.

## License

TBD

## Acknowledgments

- Inspired by JetBrains' AI agent session tracking
- Built on [Zellij](https://zellij.dev/) - a modern terminal multiplexer
- Uses the excellent [Zellij plugin API](https://zellij.dev/documentation/plugins)

