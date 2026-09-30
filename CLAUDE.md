# CLAUDE.md: CodeAtlas

Guidance for Claude Code (and other AI assistants) working in this project.

## Project Summary

A codebase intelligence engine for coding agents: AST-aware indexing, code-tuned embeddings, a symbol graph and context packing, served over MCP so Claude Code and IDEs understand huge monorepos. CodeAtlas parses repos with tree-sitter into symbols (functions, classes, modules), builds a cross-reference graph (calls, imports, inheritance, tests-for), and embeds symbols with an embedding model fine-tuned on code search pairs mined from the repo's own history (commit messages → changed functions). It exposes MCP tools (`find_relevant_code`, `get_symbol`, `callers`, `impact_of_change`, `pack_context`) that return token-budgeted, deduplicated context. Incremental indexing and multi-level caches keep it fast on every commit.

## Read First

- `PRD.md`: what we are building and why; requirement IDs (FR-xx) are referenced in commits.
- `SYSTEM_ARCHITECTURE.md`: components, data flow and failure modes.
- `TECHNICAL.md`: stack, data models, APIs and algorithms.
- `DESIGN.md`: UX flows, screens and visual tokens.
- `BRAIN.md`: concept primers, experiments and the decision log.

## Commands

| Command | What it does |
|---|---|
| `cargo build --release -p indexer` | Build indexer |
| `uv run codeatlas index ./my-repo` | Index repo |
| `uv run codeatlas serve --transport stdio` | MCP server |
| `claude mcp add codeatlas -- uv run codeatlas serve` | Add to Claude Code |
| `uv run codeatlas bench` | Benchmark |

## Code Conventions

- Symbol ids are stable and human-readable; never expose internal integer ids to agents.
- Keep functions small and typed; no silent `except`. Log with structured fields.
- Every new feature references its FR-ID in the PR description and ships with tests.
- Secrets only come from environment variables. Never hard-code keys or commit `.env`.

## Gotchas

- Generated/vendored code must be excluded or it dominates results.
- Token counts must use the client model's tokenizer when available.

## When Unsure

- Prefer the simplest design that satisfies the PRD; record bigger trade-offs in `BRAIN.md` → Decision Log.
- Run the eval / benchmark suite before claiming a quality or performance improvement.
- Ask before adding a new paid dependency, external service or data store.
