# Hierarchical Summaries – Design

This document describes the design for **hierarchical (recursive) summaries** in crumbeez. Rather than producing a flat list of summaries, the system generates short summaries for small groups of events, then summarizes those summaries at higher levels, forming a tree that users can expand and collapse for more or less detail.


## 1. Concept & Terminology

### Summary Levels

The hierarchy can be indexed by level, with level 0 being the leaves and larger numbers summarizing larger stretches of time. For example (specifics subject to change):

| Level | Example Scope | Trigger | Example |
|-------|---------------|---------|---------|
| 0 |  A small group of events (one to a few hundred events at most, one "activity burst") | Pane switch, inactivity timer, command completion, finished typing, etc. | "Edited `event_log.rs`: added `SummaryNode` struct with 4 fields; ran `cargo check`, 0 errors" |
| 1 |  A few leaf summaries covering a coherent work segment | Count threshold (N leaves accumulated) or time threshold (e.g. 30 min) | "Implemented the summary data model: added `SummaryNode` and `SummaryTree` to crumbeez‑lib, updated serialization, wrote 3 unit tests" |
| 2 |  The amount of work that would warrant a git commit | `git commit` run or a similar amount of work done | [A summary of a commit message] |
| 3 |  A pull request made or updated, or similar amount of work | `gh pr create`, PR detected from commited work, or similar amount of work done | [A summary of the PR writeup] |
| 4 |  All section summaries within a session, or a day's work | Session end, day boundary, or manual trigger | "Full‑day session: refactored summarization pipeline to support hierarchical summaries, integrated LLM backend, fixed 2 bugs in pane tracking" |

*Note:* Users may have different preferences for thresholds. Should be tunable. If LLM-driven grouping described below is used, a user prompt should be configurable. Otherwise some kind of scripting would probably be necessary in order to account for all possible preferences.

### Key Terms

- **SummaryNode** – A single summary at any level, with metadata linking it to its parent and children.
- **SummaryTree** – The in‑memory representation of the full hierarchy for the current session.
- **Leaf events** – The raw `LogEntry` items from the `EventLog` that a leaf summary covers.
- **Rollup** – The process of combining N child summaries into a parent summary at the next level.
- **Detail expansion** – An LLM requesting the full text of a child summary (or even raw events) during rollup, when the child's one‑line digest is insufficient.

### LLM-Driven Grouping

Provide the LLM a range of events or smaller summaries and let it decide how to group them:

| Aspect | Threshold-Based (current) | LLM-Driven Grouping |
|-------|---------------------------|------------------------|
| **Trigger logic** | Hardcoded thresholds (5 leaves, 30 min) | LLM decides group boundaries based on context |
| **Pros** | Deterministic, predictable; simple to implement | Handles varied scenarios naturally; leverages LLM's core strength |
| **Cons** | Brittle for edge cases; requires tuning | Non-deterministic; requires well-formed output; token cost risk |

### Alternative: Hard-coded Rollup Triggers

If LLM's end up struggling with logically delineating summaries or other issues result, we can code summary
threshold in instead, but configuration becomes much more challenging. A simple "number of levels" config could maybe work, or maybe a scripting language could be included. Either way, making an LLM work would be much simpler.

**Prompt example**:
```
You are organizing developer activity into a hierarchy.
Below are {N} sequential activity summaries from {time_range}.

Summaries:
1. <digest_1>
2. <digest_2>
...

Produce a hierarchical grouping:
1. GROUP the summaries into logical segments (3-7 per group)
2. For each GROUP output:
   - GROUP_START: <number>
   - GROUP_DIGEST: <one-line summary>
   - GROUP_BODY: <2-4 sentences>
   - GROUP_END

If you need any more deetail about a given digest, You can fallibly request it with the following syntax:

**TODO: details tool-call syntax

```


## 2. Data Model

### `SummaryNode` (new struct in `crates/crumbeez-lib/src/summary.rs`)

```rust
pub type SummaryId = String; // Format: "<session_id>-L<level>-<seq>" e.g. "a1b2c3d4-L0-007"

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryNode {
    pub id: SummaryId,
    pub level: u8,                    // 0 = leaf, 1 = section, 2 = session
    pub parent_id: Option<SummaryId>, // None for unrolled or top-level nodes
    pub children: Vec<SummaryId>,     // Empty for leaves
    pub time_start_ms: u64,
    pub time_end_ms: u64,
    pub digest: String,               // One-line digest (max ~100 chars)
    pub body: String,                 // Full Markdown summary text
    pub event_count: u32,             // Transitive event count
    pub llm_generated: bool,
    pub generation: u16,              // Incremented on re-generation via detail expansion
}
```

### `SummaryTree` (new struct in `crates/crumbeez-lib/src/summary.rs`)

```rust
#[derive(Debug, Default)]
pub struct SummaryTree {
    nodes: HashMap<SummaryId, SummaryNode>,
    roots: Vec<SummaryId>,             // Highest-level nodes, chronological order
    pending_leaves: Vec<SummaryId>,    // Leaves not yet rolled up into a section
    pending_sections: Vec<SummaryId>,  // Sections not yet rolled up into a session
    session_id: String,
    sequence_counters: [u32; 3],       // Per-level monotonic ID counters
}
```

Key methods: `add_leaf()`, `should_rollup_leaves()`, `should_rollup_sections()`, `rollup_leaves()`, `rollup_sections()`, `get_node()`, `get_children_digests()`, `get_children_bodies()`, `flatten_for_display()`.

### `DisplayNode` (for UI rendering)

```rust
pub struct DisplayNode {
    pub id: SummaryId,
    pub level: u8,
    pub depth: usize,      // Indentation depth for rendering
    pub digest: String,
    pub body: String,
    pub time_start_ms: u64,
    pub time_end_ms: u64,
    pub has_children: bool,
    pub expanded: bool,
}
```

### Relationship to Existing `Summary` Struct

The existing `Summary` struct in `event_log.rs` remains as an internal helper used only during leaf‑summary generation to compute event‑type counts for the NoOp backend. The new `SummaryNode` becomes the canonical summary type.


## 3. Storage Format

### Principles

- **Human‑readable**: Markdown summaries with understandable filenames and headers.
- **Machine‑parseable**: File names (and optionally frontmatter or other attributes) contain metadata to reconstruct the tree structure. We may not need any frontmatter at all, but it's there if useful.
- **Living**: As more events come in, the LLM may need to update recent summaries to reflect a clearer picture of what the user is doing. The last summary at each level can be re-generated to incorporate new context. The user can also edit summaries manually if necessary.

### Filesystem hierarchy example:
```
.crumbeez/summaries/2026/02/25/    # date format configurable
├── 14_00-Create 3 PRs in Bevy.md  # The coarsest summary of everything that was done in a session
└── 17_23-...
```

If a sessions lasts long enough to be unweildy in one file, more folders could be added and the files linked to in the session's markdown headers.

##### File example (`.crumbeez/summaries/2026/02/25/14_00-Create 3 PRs in Bevy.md`):

```markdown
# Created 3 Pull Requests to the Bevy repo

## 14:00..14:25 Support Schedule Commands [#23145](...)

Fixes [#23140](...)
Emulates [#23090](...)

### 14:00..14:19 Edited 2 files

- 14:00 Edited `mod.rs`: Added `schedule_commands` module </summary>

- <details>
  <summary>
    14:00 Edited `schedule_commands.rs`: implemented `ScheduleCommands` and `ScheduleCommandsExt`
  </summary>

    - 14:00:37Z Added `ScheduleCommands` struct
    - 14:02:42Z Added `ScheduleCommandsExt` trait.
    - 14:04:21Z Implemented `ScheduleCommandsExt` for `Commands`
    - ...

</details>

- <details>
  <summary> 14:15:24Z Edited `schedule_commands.rs`: Added tests </summary>

    - 14:15:24Z Added tests module
    - 14:15:58Z Implemented `...` test
    - 14:17:23Z Implemented `...` test
    - ...

</details>

### 14:19..14:23 Ran tests

- <details>
  <summary> 14:05:00Z Ran tests. All passed. </summary>

    - 14:05:00 Switch to terminal pane
    - 14:05:04 Run `cargo test` with 0 errors

</details>

### 14:23..14:25 Created PR

- <details>
  <summary> committed, pushed to GH, and made PR [#23145](...) </summary>

      - 14:23 `git commit`, edited message in external editor
      - 14:24 `git push --set-upstream origin`
      - 14:25 `gh pr create`

</details>

## 14:25..14:52 Deny missing docs for bevy_image [#23160](...)

Removed #![expect(missing_docs)] and added docs

### 14:25..14:27 ...

...

```

* Note: Some "frontmatter" may be added for metadata useful to the user or renderers, like timespans, session context, etc., but nothing strictly required for technical reasons.


### Parent Relationships

Since summaries are living documents that can be updated, parent-child relationships are stored in the parent summary (in its header or frontmatter, or just implicitly via the directory/file structure). During deserialization, `SummaryTree::load()` reads these relationships to reconstruct the tree.


## 4. Summarization Pipeline Changes

### New Flow

```
Events accumulate → trigger → generate_leaf_summary() → SummaryTree.add_leaf()
    → check should_rollup_leaves()
        → if yes: gather child digests → LLM prompt → SummaryTree.rollup_leaves()
            → check should_rollup_sections()
                → if yes: gather section digests → LLM prompt → SummaryTree.rollup_sections()
    → update summary files (living documents) → update UI
```

### Orchestrator

A new `SummarizationOrchestrator` (in `crates/zellij-plugin/src/summarization.rs`) replaces inline logic in `trigger_summary_for_pane_switch()` and the `Event::Timer` handler:

1. **Leaf generation**: Consumes unconsumed events → `SummaryNode` at level 0.
2. **Rollup decision**: After each leaf, checks thresholds (5 pending leaves OR 30 min since oldest pending leaf — both configurable).
3. **Section generation**: Collects child digests, sends to LLM, handles detail expansion.
4. **Session generation**: Same pattern over pending sections.

The orchestrator is modeled as a state machine (like `RootDiscovery`) to handle async LLM round‑trips, tracking a `RollupPhase` enum.

### LLM Prompt Templates

**Leaf prompt** (level 0):
```
You are summarizing a developer's recent activity in a terminal session.
Events: {events_formatted}
Produce:
1. DIGEST (max 80 chars): the essence of what happened.
2. BODY (2-5 sentences, Markdown): files touched, commands run, outcomes.
Format: DIGEST: <text>\nBODY:\n<markdown>
```

**Section prompt** (level 1):
```
You are creating a higher-level summary of a work segment. Below are digests:
{child_digests_numbered}
Produce DIGEST (max 100 chars) and BODY (3-8 sentences).
If any digest is too vague, respond with: NEED_DETAIL: <number>
```

**Session prompt** (level 2): Same structure as section, scoped to full session.


## 5. LLM Detail‑Request Mechanism

### Protocol

1. **Response parsing**: After the LLM returns, scan for `NEED_DETAIL: <number>` lines.
2. **Detail injection**: Fetch `SummaryTree::get_node(child_id).body` for requested children, re‑send prompt with bodies inlined.
3. **Retry limit**: Max 2 detail‑expansion rounds per rollup. After that, proceed with whatever was produced.
4. **Deeper expansion**: Session rollup can request detail on sections (gets section body); if the LLM still needs more, the second round provides the section's children's bodies (2 levels deep max).

### NoOp Backend

Never requests detail. Concatenates child digests as a bullet list for the section body; uses a prefix as the digest.


## 6. UI Interaction Model

### Summary Pane Layout

```
┌─ crumbeez — session summary ─────────────────────────────────┐
│ ▼ 14:00–15:30  Implemented hierarchical summary data model    │
│   ▼ 14:00–14:05  Edited event_log.rs: added SummaryNode      │
│     Edited `event_log.rs`: added `SummaryNode` struct...      │
│   ► 14:05–14:12  Wrote SummaryTree implementation             │
│   ► 14:12–14:20  Updated serialization format                 │
│ ► 15:30–16:15  Fixed pane content tracking bugs               │
│ [j/k] navigate  [Enter/l] expand  [h] collapse               │
└───────────────────────────────────────────────────────────────┘
```

### Expand/Collapse State

- Stored in `State` as `HashMap<SummaryId, bool>`.
- **Defaults**: Level 2 expanded, levels 0–1 collapsed.
- Ephemeral (not persisted) — UI concern only.

### Keybindings

| Key | Action |
|-----|--------|
| `j` / `↓` | Next visible node |
| `k` / `↑` | Previous visible node |
| `Enter` / `l` / `→` | Toggle expand |
| `h` / `←` | Collapse (or move to parent) |
| `Space` | Toggle expand/collapse |
| `1` / `2` / `3` | Collapse all to level N |
| `e` | Expand all |
| `g` / `G` | Jump to top / bottom |

### Rendering

`SummaryTree::flatten_for_display()` returns `Vec<DisplayNode>` respecting expand/collapse state. Each node renders as:
- **Collapsed**: `► HH:MM–HH:MM  <digest>` (one line)
- **Expanded**: `▼ HH:MM–HH:MM  <digest>` followed by indented body lines

Indentation: 2 spaces per depth level.


## 7. Impact on Existing Code

### New Files

| File | Contents |
|------|----------|
| `crates/crumbeez-lib/src/summary.rs` | `SummaryNode`, `SummaryTree`, `DisplayNode`, `SummaryId`, serialization/deserialization |
| `crates/zellij-plugin/src/summary_io.rs` | `SummaryIO`: async file read/write for summary logs (analogous to `EventLogIO`) |
| `crates/zellij-plugin/src/summarization.rs` | `SummarizationOrchestrator`: rollup logic, prompts, detail‑expansion, NoOp fallback |

### Files to Modify

| File | Changes |
|------|---------|
| `crates/crumbeez-lib/src/lib.rs` | Add `mod summary;` + `pub use summary::*;`. Add rollup threshold constants. |
| `crates/crumbeez-lib/src/event_log.rs` | `Summary` struct stays as internal NoOp helper. No structural changes. |
| `crates/zellij-plugin/src/main.rs` | (1) Add `summary_tree: SummaryTree`, `summary_io: SummaryIO` to `State`. (2) Replace `pending_summaries: Vec<String>` with tree. (3) Add `expanded_nodes: HashMap<SummaryId, bool>`, `cursor_position: usize`. (4) Refactor `trigger_summary_for_pane_switch()` to call orchestrator. (5) Refactor `Event::Timer` handler. (6) Rewrite `render()` for tree display. (7) Add key handling for tree navigation. |
| `crates/zellij-plugin/src/event_log_io.rs` | `generate_summary()` becomes a helper called by the NoOp path; no longer the entry point. |

### Dependency Order

```
summary.rs (crumbeez-lib) → lib.rs exports → summary_io.rs → summarization.rs → main.rs
```


## 8. Risks and Open Questions

### Risks

1. **Prompt engineering fragility** — The `NEED_DETAIL: <number>` protocol depends on LLM compliance. *Mitigation*: Parse leniently (regex), treat malformed responses as "no detail needed." Test with multiple models.

2. **Token budget** — Rollup prompts with many child digests/bodies may exceed context limits for small local models. *Mitigation*: Configurable max children per rollup; if exceeded, split into sub‑rollups.

3. **Large file growth** — Long sessions produce large summary files. *Mitigation*: Break large files into subdirectories using a pattern similar to Rust's `module.rs` + `module/submodule.rs`, linking sub-files in parent files.

4. **WASM constraints** — No direct filesystem access; must use `run_command` with base64 encoding (proven pattern from `EventLogIO`).

5. **Async LLM calls** — Rollup may need multiple round‑trips. Must model as state machine (like `RootDiscovery`). *Mitigation*: Orchestrator tracks a `RollupPhase` enum.

6. **UI complexity** — Tree navigation in a terminal is nontrivial. *Mitigation*: Start with simple j/k/Enter/h; defer smooth scrolling to later.

### Open Questions

1. **Session identity** — Use Zellij session name + start timestamp for human readability + uniqueness?
2. **Cross‑session continuity** — Start new file on new day; include `previous_session` reference in file header?
3. **Manual summary trigger** — Add a keybinding (e.g. `Ctrl+Enter` when crumbeez pane focused) for "summarize now"?
4. **Rollup timing** — Asynchronous (event‑driven via `WebRequestResult` / `RunCommandResult`). UI shows "⏳ Rolling up…" while waiting.
5. **Rebuild from events** — Leaves can be re‑generated from events, but section/session summaries require LLM re‑generation (may produce different text).
6. **Configurable level count** — Data model supports arbitrary levels (`level: u8`). New levels should move existing ones within themselves.


## 9. Estimated Effort

| Component | Complexity | Estimate |
|-----------|-----------|----------|
| `summary.rs` data model + tests | Medium | 3–4 hours |
| `summary_io.rs` file I/O | Medium | 2–3 hours |
| `summarization.rs` orchestrator (NoOp) | Medium | 3–4 hours |
| `summarization.rs` LLM prompts + detail expansion | High | 4–6 hours |
| `main.rs` wiring + state refactor | Medium | 2–3 hours |
| Tree UI rendering + keybindings | High | 4–6 hours |
| Doc updates (DESIGN.md, DEVELOPMENT_PLAN.md) | Low | 1 hour |
| Testing & iteration | Medium | 3–4 hours |
| **Total** | **High** | **~22–31 hours** |

### Recommended Implementation Order

1. `summary.rs` data model with unit tests (native target, no WASM needed)
2. `summary_io.rs` file I/O
3. `summarization.rs` with NoOp backend (end‑to‑end pipeline)
4. `main.rs` wiring (minimal UI: flat list of leaf digests first)
5. Tree UI rendering with expand/collapse
6. LLM prompt templates + detail expansion
7. Doc updates
