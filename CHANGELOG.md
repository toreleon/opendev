# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-03-25

### Changed

- Provider docs now include a validated setup for custom OpenAI-compatible endpoints via `api_base_url`

### Fixed

- Environment variables now override stored `api_key` config values during provider auth resolution
- Homebrew release publishing now rewrites the generated formula class name before pushing to the tap
- Homebrew install docs now cover stale tap cleanup and local-dev symlink conflicts

## [0.1.1] - 2026-03-25

### Added

- Auto-version welcome panel from Cargo.toml (no more hardcoded version strings)
- Truncation notice in write_todos result to prevent LLM retry loops
- Todo panel auto-hide lifecycle with grace period
- Spinner and Ctrl+T hint in todo panel title
- Differentiated markdown heading styles with distinct colors and underlines

### Changed

- Default reasoning effort from high to medium
- Default autonomy level from Manual to Semi-Auto
- Improved todo panel styling: green header, arrow spinner, gold completed items
- Simplified todo panel to minimal 3-color scheme
- Improved incomplete_todos_nudge to guide workflow instead of being aggressive
- Removed parentheses from tool call displays, use space-separated format
- Extracted shared tool_line builders to eliminate 7 duplicated Span blocks

### Fixed

- Scroll-up showing blank space instead of conversation history
- Plan approval appearing stuck after user selection
- Plan panel box border off-by-one causing top line overflow
- Thinking trace headers rendered inline across interleaved blocks
- Orphan parents in task watcher by keeping finished subagents as covers
- Todo creation guard that made agent passive after creating todos
- Todo nudge guard: track task intent instead of last tool name

### Removed

- GitTool (replaced with 9 missing tool display entries and standardized tool display API)
- nextest config (plain cargo test is faster for this codebase)

## [0.1.0] - 2026-03-24

### Added

- Terminal UI (TUI) built with ratatui and crossterm
- Web UI (React/Vite) with WebSocket-based real-time agent monitoring
- 9 LLM provider support: OpenAI, Anthropic, Fireworks, Google, Groq, Mistral, DeepInfra, OpenRouter, Azure OpenAI
- Per-workflow model binding across 5 slots: Normal, Thinking, Compact, Critique, VLM
- Concurrent multi-agent sessions with independent model configurations
- 30+ built-in tools: bash, edit, file ops, web, agents, LSP, symbol navigation
- MCP (Model Context Protocol) integration for dynamic tool discovery
- Session persistence and history with JSON-based storage
- Hierarchical configuration system (project > user > env > defaults)
- Context engineering with multi-stage compaction
- Cross-platform support for macOS, Linux, and Windows
- CI pipeline with 3-platform test matrix (Ubuntu, macOS, Windows)
- Release automation with cargo-dist for 5 platform targets
- Shell installer (macOS/Linux), PowerShell installer (Windows), Homebrew tap

[0.1.1]: https://github.com/opendev-to/opendev/releases/tag/v0.1.1
[0.1.2]: https://github.com/opendev-to/opendev/releases/tag/v0.1.2
[0.1.0]: https://github.com/opendev-to/opendev/releases/tag/v0.1.0
