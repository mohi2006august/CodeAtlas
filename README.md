# CodeAtlas

> A codebase intelligence engine for coding agents: AST-aware indexing, code-tuned embeddings, a symbol graph and context packing, served over MCP so Claude Code and IDEs understand huge monorepos.

**Concepts:** `MCP`, `RAG`, `Embeddings`, `Tokenization`, `Context Window`, `Caches`, `Fine Tuning`, `Vector Database`

CodeAtlas parses repos with tree-sitter into symbols (functions, classes, modules), builds a cross-reference graph (calls, imports, inheritance, tests-for), and embeds symbols with an embedding model fine-tuned on code search pairs mined from the repo's own history (commit messages → changed functions). It exposes MCP tools (`find_relevant_code`, `get_symbol`, `callers`, `impact_of_change`, `pack_context`) that return token-budgeted, deduplicated context. Incremental indexing and multi-level caches keep it fast on every commit.

## Docs

| File | Contents |
|---|---|
| [BRAIN.md](BRAIN.md) | Concept primers, research, experiments, decision log |
| [CLAUDE.md](CLAUDE.md) | Instructions for AI coding assistants |
| [DESIGN.md](DESIGN.md) | UX flows, screens, wireframes, visual language |
| [PRD.md](PRD.md) | Problem, users, requirements, metrics, roadmap |
| [SYSTEM_ARCHITECTURE.md](SYSTEM_ARCHITECTURE.md) | Components, data flow, deployment, failure modes |
| [TECHNICAL.md](TECHNICAL.md) | Stack, data models, APIs, algorithms, setup |

## Contributors

See [CONTRIBUTORS.md](CONTRIBUTORS.md). Maintained by [@mohi2006august](https://github.com/mohi2006august).
