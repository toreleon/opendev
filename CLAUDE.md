# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
# Build the entire workspace
cargo build --workspace

# Run all tests
cargo test --workspace

# Type/lint checks
cargo check --workspace
cargo clippy --workspace -- -D warnings

# Format code
cargo fmt --all

# Run a specific crate's tests
cargo test -p opendev-tui

# Run a single test by name
cargo test -p opendev-tui test_render_thinking_expanded

# Build and install release binary
cargo build --release -p opendev-cli
# Binary outputs to target/release/opendev (not opendev-cli)

# Auto-rebuild on file changes (requires cargo-watch)
cargo watch -x 'build --release -p opendev-cli'

# Web UI (React/Vite frontend)
cd web-ui && npm ci && npm run build
```

## Architecture Overview

OpenDev is a Rust workspace (edition 2024) with 21 crates under `crates/`. It is an open-source AI coding agent that spawns parallel agents, each bound to the LLM of your choice. The binary entry point is `opendev-cli`.

### Crate Map

```text
crates/
  opendev-cli         ← Binary entry point (clap CLI, dispatches to TUI/REPL/subcommands)
  opendev-tui         ← Terminal UI (ratatui + crossterm, async event loop)
  opendev-web         ← Web backend (axum + WebSocket, broadcasts agent events)
  opendev-repl        ← REPL loop, query enhancement (@file injection), message preparation
  opendev-agents      ← ReAct loop, thinking/critique phases, prompt composition
  opendev-runtime     ← Runtime services (approval, cost tracking, modes)
  opendev-config      ← Hierarchical config loading (project > user > env > defaults)
  opendev-models      ← Shared data types and models
  opendev-http        ← HTTP client, auth rotation, provider adapters (Anthropic, OpenAI, etc.)
  opendev-context     ← Context engineering (compaction stages, message validation)
  opendev-history     ← Session persistence (JSON per project, atomic writes)
  opendev-memory      ← Memory systems (embeddings, reflection, playbook)
  opendev-tools-core  ← Tool registry, BaseTool trait, dispatch
  opendev-tools-impl  ← 30+ tool implementations (bash, edit, file ops, web, agents)
  opendev-tools-lsp   ← LSP integration and language servers
  opendev-tools-symbol← AST-based symbol navigation
  opendev-mcp         ← Model Context Protocol integration
  opendev-channels    ← Channel routing
  opendev-hooks       ← Hook system
  opendev-plugins     ← Plugin manager
  opendev-docker      ← Docker runtime support
```

### Execution Flow

1. **CLI** (`opendev-cli/main.rs`) parses args, loads config via `ConfigLoader`, dispatches:
   - Non-interactive (`-p` flag): single query via `AgentRuntime`
   - Interactive (default): enters TUI via `TuiRunner`
   - Subcommands: `config setup`, `mcp`, `run ui`
2. **AgentRuntime** orchestrates `QueryEnhancer` → `ReactLoop` → `ToolRegistry`
3. **ReactLoop** (`opendev-agents/react_loop.rs`) is the core agent loop:
   - Optional thinking phase (with skip heuristic for read-only tools)
   - Optional critique + refinement (at High thinking level)
   - Action phase: LLM call with tools → tool execution → loop
   - Completion via `task_complete` tool or nudge budget exhaustion

### TUI Architecture

Async event loop with render-from-state pattern. All state in single `AppState`. No separate view layer — widgets read from state directly.

**Event sources:** terminal (crossterm), agent (mpsc channel), ticks (60ms for animations)
**Rendering:** drain ALL queued events before re-rendering (prevents UI lag)

**Layout (top to bottom):**
- Conversation (flexible) → TodoPanel (0-10) → SubagentDisplay (0-12) → ToolDisplay (8) → TaskProgress (0-1) → Input (2) → StatusBar (2)

### Provider System

All LLM providers unified through `ProviderAdapter` trait that converts to/from a common Chat Completions format. Nine providers supported: OpenAI, Anthropic, Fireworks, Google, Groq, Mistral, DeepInfra, OpenRouter, Azure OpenAI.

Each provider's models can be independently assigned to 5 workflow slots: Normal, Thinking, Compact, Critique, VLM.

### Tool System

Three layers: `BaseTool` trait → `ToolRegistry` (HashMap<name, Arc<dyn BaseTool>>) → tool implementations. Tools declare JSON Schema for LLM consumption. Parallelizable tools (read-only) can execute concurrently. `SpawnSubagentTool` is registered late to avoid circular Arc dependencies.

### Prompt Template System

91 templates in `crates/opendev-agents/templates/`, embedded at compile time via `include_str!()` in `embedded.rs`. Resolution priority: filesystem (user override) > embedded. Templates composed via `PromptComposer` with conditional sections filtered by `PromptContext`.

### Config System

Hierarchical merge: project (`.opendev/settings.json`) > user (`~/.opendev/settings.json`) > env vars > defaults. Paths centralized via `Paths` struct. Sessions scoped by working directory with encoded path (e.g., `-Users-foo-bar`).

### Web UI Backend

Axum server with WebSocket for real-time agent events. Approval/ask-user resolved via oneshot channels (non-blocking). Bridge mode allows TUI to own execution while Web UI mirrors messages.

## Key Patterns

- **Workspace dependencies**: Shared deps in root `Cargo.toml` `[workspace.dependencies]`
- **Async runtime**: Tokio with full features
- **Error handling**: `thiserror` for library errors, `anyhow` for application errors
- **Home directory**: Use `dirs-next` (not `dirs`)
- **Tests**: Use `tempfile::TempDir`, call `.canonicalize()` for macOS `/private/var` symlink resolution
- **Atomic writes**: Config, sessions, MCP config all use `.tmp` rename pattern
- **Thinking skip heuristic**: Skip thinking after read-only tool calls that all succeeded
- **Doom-loop detection**: Tracks repeated failure patterns to avoid infinite retries
- **Nudge budget**: "No tool calls" responses accepted after 3 nudges to prevent infinite loops

## Agent Design

**CRITICAL:** Never hard-code if/else branching logic to handle LLM conversation flows. The LLM must decide the next step at each turn — not static conditionals. Design agent loops so the model reasons and chooses actions dynamically.

**CRITICAL:** When crafting system prompts, never use table format. Tables are poorly parsed by LLMs and waste tokens. Use plain prose, bullet lists, or structured sections instead.

## Code Style

- Run `cargo fmt` before committing
- Run `cargo clippy` and fix all warnings
- Follow standard Rust naming conventions (snake_case functions, CamelCase types)
