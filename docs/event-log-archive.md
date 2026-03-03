# Event Log Archive Design

## Current Behavior
- Event log is a ring buffer with 10,000 events capacity
- Serialized to `/data/events.bin` on disk
- Problem: serializes ALL events including consumed ones, leading to 3MB+ files

## Desired Behavior
1. **Sliding window** - Only keep unprocessed events in memory/on disk (current ring buffer)
2. **Archive** - When events are consumed (summarized), move them to an archive file
3. **Retrievable** - Archive should be available for LLM summarizer if needed
4. **Separate from summaries** - Archives go to a different location than `./crumbeez/summaries/`

## Implementation Plan

### 1. Add Archive File Handling
- Create `EventLogIO::save_archive()` method
- Archive path: `/data/events-archive.bin` (or similar)
- Append consumed events to archive on each serialize

### 2. Modify EventLog to Support Archive
- Add `archive()` method that returns consumed events and removes them from main buffer
- Or: keep consumed events in a secondary structure that serializes separately

### 3. Wire Into Save Flow
- After `event_log.compact()`, also call `event_log_io.save_archive()` with the compacted events
- Load archive on startup (optional - depends if we need to resume from archive)

### 4. Make Archive Retrievable for LLM
- Add method to load/archive events for LLM summarization
- Could be a separate "archive mode" that sends older events to LLM if needed

## Notes
- Archive should probably be rotated or have a max size to prevent unbounded growth
- Consider compression since archives are historical/cold data
