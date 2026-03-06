<p align="center">
  <img src="logo/logo_long.png" alt="OpenDev Logo" width="400"/>
</p>

<p align="center">The open source AI coding agent for your terminal.</p>

<p align="center">
  <a href="https://pypi.org/project/opendev/"><img alt="PyPI version" src="https://img.shields.io/pypi/v/opendev?style=flat-square" /></a>
  <a href="./LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/License-MIT-yellow.svg?style=flat-square" /></a>
  <a href="https://python.org/"><img alt="Python version" src="https://img.shields.io/badge/python-%3E%3D3.10-blue.svg?style=flat-square" /></a>
</p>

<p align="center">
  <img src="figures/introduction.png" alt="OpenDev Introduction" width="800"/>
</p>

---

### Installation

```bash
# With uv (recommended)
uv pip install opendev

# With pip
pip install opendev
```

### Quick Start

```bash
# Configure your LLM providers
opendev config setup

# Start the interactive TUI
opendev

# Or start the Web UI
opendev run ui

# Single prompt (non-interactive)
opendev -p "explain this codebase"

# Resume most recent session
opendev --continue
```

### Multi-Provider Support

OpenDev is not coupled to any single provider. It supports OpenAI, Anthropic, Fireworks, Google, and any OpenAI-compatible endpoint. Different tasks (planning, execution, compaction) can each bind to a different model, letting you optimize cost and capability independently.

### MCP Integration

Dynamic tool discovery via the Model Context Protocol for connecting to external tools and data sources.

```bash
opendev mcp list
opendev mcp add myserver uvx mcp-server-sqlite
opendev mcp enable/disable myserver
```

### Development

```bash
git clone https://github.com/opendev-to/opendev.git
cd opendev
uv venv && uv pip install -e ".[dev]"
source .venv/bin/activate

# Run tests
uv run pytest

# Code quality
black opendev/ tests/ --line-length 100
ruff check opendev/ tests/ --fix
mypy opendev/

# Build the Web UI frontend
cd web-ui && npm run build
```

### Contributing

If you're interested in contributing to OpenDev, please open an issue or submit a pull request.
