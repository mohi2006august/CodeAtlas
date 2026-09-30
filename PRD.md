# CodeAtlas: Product Requirements Document

> A codebase intelligence engine for coding agents: AST-aware indexing, code-tuned embeddings, a symbol graph and context packing, served over MCP so Claude Code and IDEs understand huge monorepos.
>
> **Concepts:** `MCP`, `RAG`, `Embeddings`, `Tokenization`, `Context Window`, `Caches`, `Fine Tuning`, `Vector Database`

## 1. Overview

CodeAtlas parses repos with tree-sitter into symbols (functions, classes, modules), builds a cross-reference graph (calls, imports, inheritance, tests-for), and embeds symbols with an embedding model fine-tuned on code search pairs mined from the repo's own history (commit messages → changed functions). It exposes MCP tools (`find_relevant_code`, `get_symbol`, `callers`, `impact_of_change`, `pack_context`) that return token-budgeted, deduplicated context. Incremental indexing and multi-level caches keep it fast on every commit.

## 2. Problem Statement

Coding agents struggle in large codebases (millions of lines). Naive chunking splits functions, generic embeddings miss code semantics, grep misses conceptually related code and context windows fill with irrelevant files. Agents waste tokens exploring, and results degrade as repos grow.

## 3. Target Users & Personas

| Persona | Needs |
|---|---|
| Developer using Claude Code/IDE agents | Agents that find the right code first time. |
| Platform/DevEx team | Make agents effective across a large monorepo. |
| New hire | Understand unfamiliar code quickly. |

## 4. Goals

- Relevant-file recall@10 ≥ 0.85 on repo-specific benchmark.
- Reduce agent exploration tokens by 50%.
- Incremental re-index of a commit < 10 s.
- Support 10M+ LOC monorepos.

## 5. Non-Goals

- Being a code editor or agent itself.
- Hosting source code outside customer infra.

## 6. User Stories

- As a developer, Claude Code calls `find_relevant_code('where do we retry failed payments?')` and gets 5 exact functions with call context.
- As a developer, `impact_of_change('billing.retry_policy')` lists callers, tests and services affected.
- As DevEx, I see agent sessions using 48% fewer tokens after enabling CodeAtlas.

## 7. Functional Requirements

| ID | Priority | Feature | Description |
|---|---|---|---|
| FR-01 | P0 | AST indexing | tree-sitter for 20+ languages; symbol-level chunks with signatures and docstrings. |
| FR-02 | P0 | Symbol graph | Calls, imports, inheritance, tests-for edges; cross-language via heuristics. |
| FR-03 | P0 | Hybrid code search | BM25 over identifiers (split camelCase/snake_case) + dense embeddings + graph expansion. |
| FR-04 | P0 | MCP server | Tools + resources (`code://symbol/{id}`) over stdio/HTTP. |
| FR-05 | P1 | Code-tuned embeddings | Fine-tune on (commit message/PR title/issue → changed symbols) pairs from repo history. |
| FR-06 | P1 | Context packer | Token-budgeted packing: signatures for far context, full bodies for near; dedupe; order by dependency. |
| FR-07 | P1 | Incremental indexing + caches | Git-diff-driven updates; embedding cache by content hash; query cache per commit. |
| FR-08 | P2 | Code-aware tokenization analysis | Measure token cost per language; suggest compact representations (signature-only views). |

## 8. Non-Functional Requirements

- Runs fully on-prem; no code leaves the network.
- Query p95 < 300 ms.
- Index freshness < 1 min after push.

## 9. Success Metrics

| Metric | Target |
|---|---|
| Recall@10 (repo benchmark) | ≥ 0.85 |
| Agent token reduction | ≥ 50% |
| Agent task success (SWE-style tasks) | +15 pts |
| Incremental index time/commit | < 10 s |

## 10. Release Plan

| Phase | Scope |
|---|---|
| M1 | tree-sitter indexing + BM25 + MCP find/get tools. |
| M2 | Symbol graph + dense embeddings + hybrid search. |
| M3 | Context packer + incremental indexing + caches. |
| M4 | Repo-specific embedding fine-tuning + benchmark. |
| M5 | Monorepo scale (sharding) + impact analysis. |

## 11. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Fine-tuning on history overfits to old code | Time-based split (train on older commits, evaluate on newer ones). |
| Graph inaccuracy in dynamic languages | Combine static analysis with runtime traces/test coverage where available. |
| Index lag in fast-moving repos | Incremental updates, priority queue for recently touched files. |

## 12. Open Questions

- Use LSP servers for precise references vs tree-sitter heuristics?
- Expose editing tools or stay read-only?
