# Hierarchical Summaries – Design

This document describes the design for **hierarchical (recursive) summaries** in crumbeez. Rather than producing a flat list of summaries, the system generates short summaries for small groups of events, then summarizes those summaries at higher levels, forming a tree that users can expand and collapse for more or less detail.


## 1. Concept & Terminology

### Summary Levels

The hierarchy is indexed by level, with level 0 (leaves/actions) at the bottom and higher levels summarizing larger stretches of activity. The exact level boundaries and triggers are intentionally flexible — the LLM decides when to create new summary levels based on the content, not hard thresholds.

*Note:* The table below is a rough illustration of how one user's session might map to levels, not a specification to implement. Different users, workflows, and time scales will produce different groupings.

| Level | Example Scope |
|-------|---------------|
| 0 | Actions — a single command run, a brief editing burst, opening a file. (Technical term: "leaf") |
| 1 | A few actions forming a logically distinct task |
| 2 | A larger work segment (e.g., what might warrant a commit) |
| 3 | A coherent body of work (e.g., a pull request, a feature implementation) |
| 4 | Session-level or day-level summary |

### Key Terms

- **Action** – The smallest unit of activity: a single command run, a brief editing burst, opening a file. This is the user-facing term for level 0 summaries. (Technical equivalent: "leaf")
- **Leaf** – Technical term for a level 0 summary node in the tree structure. Synonymous with "action" from the user's perspective.
- **SummaryNode** – A single summary at any level, with metadata linking it to its parent and children.
- **SummaryTree** – The in‑memory representation of the full hierarchy for the current session.
- **Rollup** – The process of combining N child summaries into a parent summary at the next level.
- **Detail expansion** – An LLM requesting the full text of a child summary (or even raw events) during rollup, when the child's one‑line digest is insufficient.

### LLM-Driven Grouping

The system uses the LLM to determine when to create new summary levels and what boundaries to use. Instead of hard thresholds (e.g., "5 items" or "30 minutes"), the LLM decides based on whether activities form a **logically distinct task**.

#### What makes tasks "logically distinct"?

A boundary between tasks exists where a human would say "that was one thing, now I'm on another." Consider:

- **Context switches**: different files, modules, projects, or goals
- **Task completion**: a build/test finished, a commit made, a document saved
- **Semantic shifts**: moved from implementing → debugging → reviewing
- **Command sequences**: related commands that accomplish one goal (e.g., edit config → reload service → verify)

The LLM applies these heuristics organically rather than following rigid rules.

#### Safety Maximums

To prevent runaway token usage and ensure responsiveness:

- Maximum ~50 actions per grouping prompt
- Maximum ~10 groups per rollup response

These limits are **not** mentioned in the LLM prompt. Instead, if the LLM exceeds these limits, the system detects it and provides feedback:

> "You produced N groups, which is an order of magnitude more than expected. Please reconsider and combine adjacent activities that form a single task."

This feedback mentions "order of magnitude" rather than exact numbers to avoid biasing the LLM toward a specific count.

#### Grouping Prompt Template

```
You are grouping terminal activity into logically distinct tasks.
A "logically distinct" task is where a human would say "that was one thing, now I'm on another."

Group these actions. For each group, output:
GROUP <start_idx>-<end_idx>: <2-5 word task label>

Example:
GROUP 0-12: Configure user authentication
GROUP 13-18: Write unit tests
GROUP 19-25: Update documentation

Now process these actions:
{formatted_actions}
```

The response format is simple (just indices and labels) to minimize parsing complexity. If the LLM needs more detail about any action to make a grouping decision, it can request it using the detail mechanism described below.


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
├── 14_00-Configure backup system.md  # The coarsest summary of everything that was done in a session
└── 17_23-...
```

If a sessions lasts long enough to be unweildy in one file, more folders could be added and the files linked to in the session's markdown headers.

##### File example (`.crumbeez/summaries/2026/02/25/14_00-Configure backup system.md`):

```markdown
# Configured automated backup system for production servers

## 14:00..14:25 Set up backup scripts

Installed rsync-based backup to remote storage.

### 14:00..14:19 Created backup script

- 14:00 Edited `backup.sh`: Added rsync commands with preserve flags

- <details>
  <summary>
    14:00 Edited `backup.sh`: implemented rotation logic
  </summary>

    - 14:00:37Z Added rotation function
    - 14:02:42Z Added date-based naming.
    - 14:04:21Z Implemented retention policy
    - ...

</details>

- <details>
  <summary> 14:15:24Z Edited `backup.sh`: Added tests </summary>

    - 14:15:24Z Added test cases for rotation
    - 14:15:58Z Implemented rsync dry-run test
    - 14:17:23Z Added error handling tests
    - ...

</details>

### 14:19..14:23 Tested scripts locally

- <details>
  <summary> 14:05:00Z Ran backup script. All files copied successfully. </summary>

    - 14:05:00 Switch to terminal pane
    - 14:05:04 Run `./backup.sh --dry-run` with 0 errors

</details>

### 14:23..14:25 Deployed to cron

- <details>
  <summary> 14:23 Added backup to crontab </summary>

      - 14:23 `crontab -e`, added daily backup entry
      - 14:24 `systemctl list-timers` to verify
      - 14:25 Confirmed timer will run at 02:00

</details>

## 14:25..14:52 Documented restore procedure

Created README with restore instructions.

### 14:25..14:27 ...

...

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

1. **Action/leaf generation**: Consumes unconsumed events → `SummaryNode` at level 0.
2. **Rollup decision**: After each leaf, the orchestrator checks if there are enough pending items to warrant a grouping prompt (safety limits). If so, it sends the pending items to the LLM for grouping.
3. **Section generation**: Collects child digests, sends to LLM for rollup, handles detail expansion.
4. **Session generation**: Same pattern over pending sections.

The orchestrator is modeled as a state machine (like `RootDiscovery`) to handle async LLM round‑trips, tracking a `RollupPhase` enum.

### LLM Prompt Templates

**Action/leaf prompt** (level 0):
```
You are summarizing a user's recent terminal activity.
Actions: {events_formatted}
Produce:
1. DIGEST (max 80 chars): the essence of what happened.
2. BODY (2-5 sentences, Markdown): files touched, commands run, outcomes.
Format: DIGEST: <text>\nBODY:\n<markdown>
```

**Section prompt** (level 1):
```
You are creating a higher-level summary of a work segment. Below are digests of logically distinct tasks:
{child_digests_numbered}
Produce DIGEST (max 100 chars) and BODY (3-8 sentences).
If any digest is too vague to summarize confidently, respond with: NEED_DETAIL: <number>
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
│ ▼ 14:00–15:30  Configured automated backup system              │
│   ▼ 14:00–14:05  Created backup script with rotation           │
│     Created backup.sh with rsync commands...                   │
│   ► 14:05–14:12  Tested scripts locally                         │
│   ► 14:12–14:20  Documented restore procedure                   │
│ ► 15:30–16:15  Set up monitoring for backup jobs               │
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
| `crates/crumbeez-lib/src/lib.rs` | Add `mod summary;` + `pub use summary::*;`. Add safety limit constants (max groups, max items per prompt). |
| `crates/crumbeez-lib/src/event_log.rs` | `Summary` struct stays as internal NoOp helper. No structural changes. |
| `crates/zellij-plugin/src/main.rs` | (1) Add `summary_tree: SummaryTree`, `summary_io: SummaryIO` to `State`. (2) Replace `pending_summaries: Vec<String>` with tree. (3) Add `expanded_nodes: HashMap<SummaryId, bool>`, `cursor_position: usize`. (4) Refactor `trigger_summary_for_pane_switch()` to call orchestrator. (5) Refactor `Event::Timer` handler. (6) Rewrite `render()` for tree display. (7) Add key handling for tree navigation. |
| `crates/zellij-plugin/src/event_log_io.rs` | `generate_summary()` becomes a helper called by the NoOp path; no longer the entry point. |

### Dependency Order

```
summary.rs (crumbeez-lib) → lib.rs exports → summary_io.rs → summarization.rs → main.rs
```


## 8. Risks and Open Questions

### Risks

1. **LLM grouping inconsistency** — The LLM may produce unexpected or inconsistent grouping boundaries between runs. *Mitigation*: Provide clear criteria in prompts, use few-shot examples, log outputs. for debugging Consider allowing users to manually adjust boundaries.

2. **Prompt engineering fragility** — The `NEED_DETAIL: <number>` protocol depends on LLM compliance. *Mitigation*: Parse leniently (regex), treat malformed responses as "no detail needed." Test with multiple models.

3. **Token budget** — Rollup prompts with many child digests/bodies may exceed context limits for small local models. *Mitigation*: Safety limits on max items per prompt; if exceeded, split into sub‑rollups.

4. **Large file growth** — Long sessions produce large summary files. *Mitigation*: Break large files into subdirectories using a pattern similar to Rust's `module.rs` + `module/submodule.rs`, linking sub-files in parent files.

5. **WASM constraints** — No direct filesystem access; must use `run_command` with base64 encoding (proven pattern from `EventLogIO`).

6. **Async LLM calls** — Rollup may need multiple round‑trips. Must model as state machine (like `RootDiscovery`). *Mitigation*: Orchestrator tracks a `RollupPhase` enum.

7. **UI complexity** — Tree navigation in a terminal is nontrivial. *Mitigation*: Start with simple j/k/Enter/h; defer smooth scrolling to later.

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
