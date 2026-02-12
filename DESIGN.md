# Zellij Session Tracker – Design

## 1. Project Intent

Zellij Session Tracker is a **Zellij plugin** that automatically captures and summarizes what happened during your development sessions. It observes activity across panes and tabs (editing files, running tests, builds, git actions), converts that into **semantic events**, and organizes that activity into **human‑readable summaries** aligned with logical tasks that you can review later.

The design prioritizes:

1. **Automatic tracking** – Always‑on, low‑friction; no manual note‑taking.
2. **Semantic understanding** – Structured events instead of raw PTY logs.
3. **Human‑readable logs** – Platform‑agnostic log files as the canonical artifact, with UI layers as views over those logs.
4. **Multi‑pane awareness** – Understanding workflows across tabs and panes.
5. **Crash resilience** – Persisting events/summaries so you don’t lose context.
6. **Configurable LLM backends** – User‑chosen local or cloud LLMs, or none.
7. **Code‑aware context** – Optional integration with Augment Code Context Engine via MCP for richer, code‑centric summaries.


## 2. Requirements & Design Principles

### 2.1 Functional Requirements

- **Event collection**
  - Track key activity inside Zellij: pane/tab focus, commands, file changes, test/build runs, git actions (via commands).
- **Summarization**
  - Produce concise narratives of what was done, primarily aligned with logical units of work (e.g., a command or test run completing, a commit, or a coherent cluster of edits).
  - For very long‑running tasks or idle stretches with unsummarized activity, emit time‑based checkpoints so context isn’t lost after a crash.
  - Summaries should include *what* changed, *where*, and *why* as far as can be inferred.
- **Human‑consumable artifact**
  - Summaries and, optionally, higher‑level events are written to **append‑only log files** in a stable, textual format (e.g., Markdown or JSONL).
  - These logs are the **primary product**; the database layer, if used, is an implementation detail.
- **UI**
  - Provide a Zellij UI (pane/status bar) that displays summaries and basic status, backed by the same log format.
  - The system should work conceptually even if a different UI reads the logs.

### 2.2 LLM & Backend Requirements

- **User‑choice first**
  - On first use, the plugin should ask the user which summarization mode they want:
    - Local LLM (e.g., Ollama).
    - Cloud provider (e.g., OpenAI, Anthropic).
    - No LLM (record events and logs only).
  - This choice is persisted and can be changed later via configuration or a simple settings view.

- **Backend abstraction**
  - Summarization goes through a `SummarizationBackend` abstraction that hides implementation details of specific providers.
  - Multiple backends can be supported without changing core summarization logic.

### 2.3 Storage & Persistence Requirements

- **Log‑first design**
  - The canonical representation of what happened is a set of **append‑only logs** on disk:
    - **Event logs** (structured but not necessarily human‑friendly).
    - **Summary logs** (explicitly human‑friendly, e.g., Markdown or structured text).
- **Pluggable indexing**
  - An optional **index store** (e.g., SQLite) can be used for fast querying, filtering, and correlation, but it is *not* the primary source of truth.
  - The design must allow alternative indexing/storage implementations in the future.
- **Crash resilience**
  - Writes should be durable enough that crashes do not lose events that were already observed.
  - Log writes should be append‑only and recoverable.

### 2.4 Context & Integrations

- **Augment Code Context Engine (MCP)**
  - The plugin can optionally call out to an Augment MCP endpoint to enrich summaries with:
    - Codebase structure and semantics.
    - Summaries of diffs or changed files.
    - Cross‑file or project‑wide relationships.
  - This should be handled via a `ContextProvider` abstraction so additional context sources can be added later.

- **External tools**
  - Design logs such that other tools (e.g., scripts, editors, web UIs) can consume them without intimate knowledge of the plugin internals.


## 3. Architecture Overview

At a high level, the system looks like this:

```
Zellij Events
    ↓
Event Collector & Event Model
    ↓
Event Log Writer  ──────▶  Append‑only Event Logs (canonical)
    │
    ├──────────────▶  Optional Index Store (e.g., SQLite)
    ↓
Summarization Orchestrator (Task/checkpoint‑based)
    ↓                      ↘
Context Providers      Summarization Backend(s)
(Augment MCP, git, …)       (Local / Cloud / None)
    ↓                           ↓
Enriched Summary Context  ──────┘
    ↓
Summary Formatter & Log Writer  ▶  Human‑Readable Summary Logs (canonical)
    ↓
UI Layer(s) (Zellij pane, status bar, external tools)
```

The Zellij plugin is responsible for event collection, orchestration, and providing at least one built‑in UI. Storage, summarization, and context providers are structured as replaceable components.


## 4. Zellij Plugin Responsibilities

The plugin is compiled to WASM and runs inside Zellij, using the `zellij_tile` API.

Core responsibilities:

1. **Permissions & subscription**
   - Request permissions such as `ReadApplicationState`, `RunCommands`, and `WebAccess`.
   - Subscribe to relevant events: `PaneUpdate`, `TabUpdate`, `FileSystemUpdate`, `Timer`, `RunCommandResult`, and optionally `Key` events.

2. **Event collection & modeling**
   - Maintain an in‑memory view of panes, tabs, and sessions.
   - Transform raw Zellij events into internal semantic events (e.g., `EditorSessionStarted`, `TestRunCompleted`, `GitCommitRecorded`).

3. **Persistence orchestration**
   - Append events to the event log.
   - Optionally update an index store for efficient querying.

4. **Summarization orchestration**
   - Detect logical task boundaries (e.g., commands, test runs, commits) and trigger summarization for those chunks of activity.
   - Use `Timer` events as a safety net to flush in‑progress activity if it has gone unsummarized for too long.
   - Coordinate fetching of context (Augment MCP, git, etc.) and calling the configured `SummarizationBackend`.

5. **UI rendering**
   - Render a basic “session tracker” pane showing summaries and status.
   - Optionally provide status‑bar indicators and simple navigation (paging/scrolling summaries).


## 5. Event Model

The internal event model is intentionally simple and extensible. Examples include:

- `PaneFocused { pane_id, tab_id, timestamp }`
- `PaneCommandChanged { pane_id, command, timestamp }`
- `FileModified { path, pane_id?, tab_id?, timestamp }`
- `EditorSession { pane_id, tool, files, start, end }`
- `TestRun { command, status, passed, failed, duration, timestamp }`
- `BuildRun { command, status, errors_summary, timestamp }`
- `GitCommit { hash, message, files_changed, timestamp }`

These events are:

- Serialized into an **event log** (e.g., JSONL with one event per line).
- Optionally mirrored into an index store for queryability.

Program‑specific handlers (editor/test/build/git) are responsible for recognizing relevant patterns in pane state and `RunCommandResult` output, and emitting these higher‑level events.


## 6. Storage & Logs

### 6.1 Event Logs (Canonical)

- Format: structured, line‑oriented (e.g., JSONL) so tools can parse it easily.
- Location: within a configurable data directory, e.g., `~/.local/share/zellij-session-tracker/events/`.
- Properties:
  - Append‑only; never mutated in place.
  - Segmented by date and/or session to keep files manageable.
  - Recoverable after crashes (at worst, the last partial line is discarded).

### 6.2 Summary Logs (Canonical)

- Format: **human‑readable text**, typically Markdown or structured plain text, for example:
  - Time window (e.g., `2026‑02‑10 14:00‑14:10`).
  - High‑level description of what changed.
  - Key files and commands involved.
  - Optional notes about failures, TODOs, or follow‑ups.
- Location: similar to event logs, e.g., `~/.local/share/zellij-session-tracker/summaries/`.
- The Zellij UI will read from these logs to display summaries, but *any* other tool can also read them.

### 6.3 Optional Index Store (e.g., SQLite)

- Purpose:
  - Fast queries over long history (e.g., “summaries for repo X in the last week”, “all events involving file Y”).
  - Efficient correlation across logs.
- Requirements:
  - Treat as a **cache / index**, not the canonical store.
  - Rebuildable from logs if necessary.
  - Pluggable: the initial implementation may use SQLite, but the design allows replacing or extending it.


## 7. Summarization Pipeline

### 7.1 Scheduling

- The summarization orchestrator is primarily **task‑driven**:
  - When it observes that a logical unit of work is complete (e.g., a test run or build finishes, a git commit is recorded, or we identify a coherent stretch of edits), it groups the corresponding events and triggers summarization for that chunk.
- `Timer` events are used as a **safety mechanism**:
  - If there is unsummarized activity for longer than a configurable threshold, a checkpoint summary is emitted so that progress is still captured even if Zellij crashes or the session ends unexpectedly.
- When a summarization run is triggered (task boundary or safety timer), the plugin:
  1. Determines which events have not yet been summarized for the current session/window.
  2. Builds a compact representation of those events.
  3. Fetches additional context (Augment MCP, git, etc.).
  4. Calls the configured `SummarizationBackend`.
  5. Writes the result to the summary log.

### 7.2 Context Providers (including Augment MCP)

- A `ContextProvider` abstraction:
  - Accepts a set of inputs (e.g., repo root, files touched, git commit hash).
  - Returns structured context: file summaries, related files, semantic tags, etc.
- Augment Code Context Engine MCP is a primary implementation:
  - The plugin communicates with a local MCP endpoint (via `web_request` or `run_command`, depending on integration) to request context for relevant files.
  - The response is included (possibly truncated or summarized) in the data passed to the `SummarizationBackend`.
- Other potential context sources: `git status/diff`, project metadata, test/build tooling.

### 7.3 Summarization Backend Abstraction

- The plugin calls a `SummarizationBackend` interface that:
  - Receives: structured event summary + optional context.
  - Returns: a human‑readable summary string (plus metadata if needed).
- Implementations:
  - **LocalLLMBackend** – e.g., Ollama or another local server.
  - **CloudLLMBackend** – OpenAI, Anthropic, etc.
  - **NoOpBackend** – returns a simple, locally‑generated summary (or none) when the user chooses “no LLM.”


## 8. First‑Run Experience & Configuration

### 8.1 First‑Run Onboarding

On first launch (no existing config/state), the plugin should:

1. Detect that no summarization backend has been configured.
2. Present a minimal onboarding UI:
   - Explain that the plugin can use either a local model, a cloud provider, or no LLM.
   - Offer a simple choice: `Local`, `Cloud`, or `None`.
3. Collect any minimal required information (e.g., endpoint URL or an environment variable name for API keys, but not the secret itself).
4. Persist this choice in configuration or plugin state.

### 8.2 Configuration Model (Conceptual)

Configuration fields (conceptually):

- **Storage**
  - `data_dir`: where event/summary logs live.
  - `index_backend`: `none` / `sqlite` / other.
- **Summarization**
  - `enabled`: `true` / `false`.
  - `max_unsummarized_minutes`: optional safety threshold for emitting checkpoint summaries during long‑running tasks.
  - `backend`: `local`, `cloud`, or `none`.
  - Backend‑specific fields (model name, endpoint URL, env var keys).
- **Context Integrations**
  - `augment_mcp_enabled`: `true` / `false`.
  - Connection parameters for the MCP endpoint.

Configuration can be read from Zellij plugin config (e.g., `config.kdl`) and/or a separate config file under `data_dir`.


## 9. UI & Summary Display

The UI’s job is to **present** information that already exists in logs or in memory. It should not own unique, non‑logged state.

Core UI concepts:

- **Summary Pane**
  - A dedicated pane that shows the most recent summary and allows paging through previous summaries.
  - Renders directly from the summary log format (Markdown/plain text).
- **Status Indicator**
  - Optional status bar widget showing whether tracking is active, time since last summary, and basic stats.
- **Minimal Interaction**
  - Basic keybindings for next/previous summary, jump to “current” summary, and opening settings/onboarding again.

Because the underlying format is log‑centric and human‑readable, future UIs (e.g., editor integrations or web dashboards) can reuse the same logs.


## 10. Development Roadmap (High‑Level)

This roadmap is intentionally high‑level; **concrete setup and step‑by‑step instructions live in a separate implementation guide** (the `DEVELOPMENT_PLAN.md` implementation guide). Phases 1–3 roughly correspond to the MVP (v0.1), while Phases 4–5 are post‑v0.1 evolution.

- **Phase 1 – Foundations**
  - Scaffold the Zellij plugin project and establish a dev workflow.
  - Implement a minimal “hello world” UI.
  - Define configuration shape for storage, summarization backend, and context integrations.
- **Phase 2 – Event Collection & Logging**
  - Subscribe to core events and design the event model.
  - Implement append‑only event logging and, optionally, a simple index store.
- **Phase 3 – Summaries, Logs, and Basic UI (MVP)**
  - Add task‑driven summarization orchestration with optional time‑based safety checkpoints.
  - Implement the `SummarizationBackend` abstraction and initial backend(s).
  - Implement first‑run onboarding and generate human‑readable summary logs.
  - Implement a basic summary pane UI that reads from those logs.
- **Phase 4 – Context & Program‑Specific Intelligence**
  - Add program‑specific handlers for editors, tests, builds, and git.
  - Integrate Augment Code Context Engine MCP via a `ContextProvider` abstraction.
  - Enrich summaries with code‑aware context.
- **Phase 5 – Advanced UX & Integrations**
  - Add query capabilities over past summaries/events (likely via the index store).
  - Implement additional UIs (timeline view, lightweight session replay, exports).
  - Explore integrations with external tools (issue trackers, knowledge bases) based on log output.


## 11. Non‑Goals (Initial Design)

- **General PTY wrapper** – The system is not intended to replace general PTY wrappers or capture raw terminal I/O for arbitrary terminals.
- **Non‑Zellij environments** – Initial scope is limited to Zellij sessions.
- **LLM provider lock‑in** – The design avoids tying core logic to any specific LLM provider.


## 12. Relationship to Other Documents

- **Architecture alternatives**
  - Detailed comparison of PTY wrapper vs shell integration vs Zellij plugin vs hybrid lives in `APPROACHES_COMPARISON.md`.
- **Zellij API capabilities**
  - `ZELLIJ_API_ANALYSIS.md` provides a deeper survey of the plugin API.
- **Implementation guide / setup**
  - `DEVELOPMENT_PLAN.md` is the single implementation guide containing concrete instructions for setting up the Rust project, building, running, and iterating on the plugin, along with a more detailed phase‑by‑phase development plan.
  - DESIGN.md remains focused on long‑term architecture, behavior, and constraints rather than step‑by‑step instructions.
