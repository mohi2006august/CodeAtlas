# CodeAtlas: Technical Specification

> A codebase intelligence engine for coding agents: AST-aware indexing, code-tuned embeddings, a symbol graph and context packing, served over MCP so Claude Code and IDEs understand huge monorepos.
>
> **Concepts:** `MCP`, `RAG`, `Embeddings`, `Tokenization`, `Context Window`, `Caches`, `Fine Tuning`, `Vector Database`

## 1. Tech Stack

| Layer | Choice | Why |
|---|---|---|
| Parsing | tree-sitter (Rust bindings) | Fast, multi-language. |
| Core | Rust (indexer) + Python (ML, MCP server) | Speed + ecosystem. |
| Embeddings | Code embedding model (e.g. jina-code, nomic-embed-code) + sentence-transformers fine-tune | Code semantics. |
| Search | Tantivy + LanceDB | Embedded, fast. |
| MCP | Python SDK | Protocol. |

## 2. Repository Layout

```text
codeatlas/
├── indexer/        # rust: parse, symbols, graph, incremental
├── search/         # bm25, dense, fusion, graph_expand
├── packer/         # budgeted rendering
├── finetune/       # mine_pairs.py, train.py, eval_timesplit.py
├── mcp_server/     # tools.py, resources.py
└── bench/          # repo-specific retrieval benchmark, agent token study
```

## 3. Data Models

```python
class Symbol(BaseModel):
    id: str                        # "py:billing/retry.py::RetryPolicy.next_delay"
    kind: Literal["function", "method", "class", "module"]
    path: str
    start_line: int
    end_line: int
    signature: str
    doc: str | None
    body_hash: str
    tokens: int

class Edge(BaseModel):
    src: str
    dst: str
    type: Literal["calls", "imports", "inherits", "tests", "references"]

class PackedContext(BaseModel):
    indexed_commit: str
    items: list[dict]              # {symbol_id, render: "full"|"signature"|"name", text}
    used_tokens: int
```

## 4. API Surface

| Method | Endpoint / Tool | Purpose |
|---|---|---|
| tool | find_relevant_code(query, budget_tokens=6000) | Packed relevant symbols. |
| tool | get_symbol(id, include='body') | Symbol source. |
| tool | callers(id, depth=2) / callees(id) | Graph navigation. |
| tool | impact_of_change(id) | Callers, tests, services affected. |
| tool | pack_context(symbol_ids, budget_tokens) | Render a custom set within budget. |
| resource | code://symbol/{id} | Symbol by URI. |

## 5. Core Algorithms

### Training pairs from history

For each commit/PR: text = title + message (+ linked issue); positives = symbols whose bodies changed (excluding formatting-only diffs). Hard negatives are BM25-similar unchanged symbols in the same files. Split by time to avoid leakage.

### Graph expansion

Seed with top-k fused results, run personalised PageRank (α=0.15, 2 hops) over call/import/test edges, and mix `final = 0.7·fused + 0.3·ppr`.

### Budgeted rendering

Sort by score. The top items get full bodies, their neighbours get signatures + docstrings, and distant items get names only. This is greedy under the token budget using exact tokenizer counts (code tokenizes very differently from prose).

### Incremental indexing

Only re-parse changed files; symbol ids are stable across moves via (path, qualified name) with a rename-detection fallback on body hash.

## 6. Configuration

| Variable | Purpose |
|---|---|
| CODEATLAS_REPO | Repo path/URL |
| CODEATLAS_EMBED_MODEL | Embedder |
| CODEATLAS_DB | Symbols DB |
| GIT_HOST_TOKEN | Permissions + webhooks |

## 7. Testing Strategy

- Parser golden tests per language.
- Time-split retrieval benchmark (train ≤ T, test > T).
- Agent A/B: token usage and task success with/without CodeAtlas on 50 internal tasks.
- Incremental index consistency vs full re-index.

## 8. Performance Budgets

- Initial index 1M LOC < 15 min (GPU embed).
- Query p95 < 300 ms.
- Incremental commit < 10 s.

## 9. Local Setup

```bash
cargo build --release -p indexer
uv run codeatlas index ./my-repo
uv run codeatlas serve --transport stdio
claude mcp add codeatlas -- uv run codeatlas serve
uv run codeatlas bench
```
