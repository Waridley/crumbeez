# ⭐ Project Reorganization Plan: Zellij Plugin Architecture to Distributed Service

**Goal:** Decouple the operational core from the Zellij Plugin client by introducing a dedicated, network-accessible Server Daemon. The Plugin will become a thin gRPC client.

**Chosen Architecture:** Client-Server model using gRPC for type-safe communication over a network (e.g., LAN, dedicated service).

## 1. High-Level Components

| Component | Role | Technology/Library | Current Location |
| :--- | :--- | :--- | :--- |
| **[CLIENT] Zellij Plugin** | The interface layer. Responsible for capturing raw events/state changes from Zellij, making gRPC calls, and relaying the resulting action/feedback back to the TUI. | Rust, `tonic` (gRPC Client) | `crates/zellij-plugin/src` |
| **[SERVER] Daemon Service** | The brain. Hosts all core library logic (`event_log`, `pane_content`, `summary`, `llm` orchestration). Listens for incoming client requests. | Rust, `tonic` (gRPC Server) | New/Refactored `crumbeez-service` crate |
| **[API] Protobuf Definition** | The contract. Defines the data structures (e.g., `KeystrokeEvent`, `LogEntry`) and the service methods callable by both client and server. | `.proto` files | New `proto` directory |

## 2. The gRPC Contract (`.proto` Definition)

The most critical step is defining the common API contract. This file governs all data exchange. We will define key service methods and message types.

**Key Message Structures to Define:**

*   `KeystrokeEvent`: Details of a single user input (timestamp, key code, text representation).
*   `PaneState`: Structured data including content, dimensions, and context for a specific pane.
*   `LogEntry`: Structured data capturing a sequence of events, including required metadata (e.g., checksum, sequence ID).
*   `SummaryPayload`: A structured message containing the derived analysis (e.g., `total_events: i32`, `event_counts: map<string, i32>`).

**Key Service Methods (`Service`):**

| Method Name | Direction | Purpose | Existing Implementation |
| :--- | :--- | :--- | :--- |
| **`StreamEvents`** | *Client $\to$ Server* | Used by the plugin to stream raw state changes (keystrokes, messages) to the server in real-time. (Streaming required for efficiency). | `EventLog::append(...)` |
| **`GetUnconsumedState`** | *Client $\to$ Server* | Requests the server to analyze the logged state and return the list of pending, unhandled events. | `EventLog::unconsumed()` |
| **`ProcessEventStream`** | *Client $\to$ Server* | An overarching or batch call that takes a batch of raw data and tells the server to process it, potentially returning a summary or confirmation. | Core logic spanning `event_log.rs` and `pane_content`. |
| **`FetchSummary`** | *Client $\to$ Server* | Requests a computed summary (e.g., "how many logs were generated over the last hour?"). | `Summary::from_events(...)` |

## 3. Implementation Phases (Execution Plan)

### Phase A: Contract Definition ( READ-ONLY )
1.  Create the top-level `.proto` file and define the messages and services outlined above.
2.  Run tooling (e.g., `protoc`) to generate the necessary client/server stub code in Rust.

### Phase B: Server Implementation (Daemon)
1.  **Refactor `crumbeez-lib`:** The contents of `event_log.rs`, `pane_content/`, and `llm/` will be moved into the new dedicated server service (e.g., `crumbeez-service`).
2.  **Implement Server Listener:** Write the boilerplate to run the gRPC server in a background process.
3.  **Implement Business Logic:** Update the service handler methods to implement the server-side logic using the refactored library components.

### Phase C: Client Implementation (Plugin)
1.  **Remove Logging/State:** Delete the local state management/persistence logic from `event_log.rs` and related plugin files, as this logic now belongs on the server.
2.  **gRPC Client Integration:** Implement a thin client module in the plugin that handles connection management, serialization, and making calls to the external server service.
3.  **Refactor Plugin Core:** Update the main plugin loop (`main.rs`) to intercept events and pass them immediately to the gRPC client instead of processing them internally.

## 4. Verification and Testing
*   **Unit Testing:** Focus on the server logic first. Write unit tests for `crumbeez-service` using mocked internal inputs to verify correct state transitions, summary generation, and API payloads.
*   **Integration Testing:** Test the full loop: Plugin $\xrightarrow{\text{gRPC Call}}$ Server $\xrightarrow{\text{Process}}$ Result $\xrightarrow{\text{gRPC Response}}$ Plugin Feedback.