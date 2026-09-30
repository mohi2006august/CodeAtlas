# CodeAtlas: Design Document

> A codebase intelligence engine for coding agents: AST-aware indexing, code-tuned embeddings, a symbol graph and context packing, served over MCP so Claude Code and IDEs understand huge monorepos.
>
> **Concepts:** `MCP`, `RAG`, `Embeddings`, `Tokenization`, `Context Window`, `Caches`, `Fine Tuning`, `Vector Database`

## 1. Design Principles

- Return less, but exactly right.
- Always report the indexed commit so agents know freshness.

## 2. Key User Flows

### Agent-assisted change

1. Developer asks Claude Code to change retry logic.
2. Claude calls find_relevant_code → gets RetryPolicy + callers + tests.
3. Calls impact_of_change.
4. Edits the right files; runs listed tests.

## 3. Screens & Surfaces

| Screen | What it shows / does |
|---|---|
| Index status (web) | Repos, indexed commit, lag, symbol counts. |
| Search playground | Query → ranked symbols with scores and graph. |
| Token savings report | Agent sessions with/without CodeAtlas. |

## 4. Wireframe (Primary Screen)

```text
find_relevant_code("where do we retry failed payments?", budget=6000)  @ a1b9e3f
 1. billing/retry.py::RetryPolicy.next_delay        full   412 tok  (score .93)
 2. billing/charge.py::charge_with_retry            full   655 tok  (.90)
 3. billing/retry.py::RetryPolicy                   sig     38 tok  (.84)
 4. tests/billing/test_retry.py::test_backoff_caps  sig     27 tok  (tests→1)
 5. workers/dunning.py::DunningJob.run              sig     44 tok  (calls→2)
 packed 1,176 / 6,000 tokens · cache miss · 142 ms
```

## 5. Visual Language

| Token | Value |
|---|---|
| Primary accent | `#475569` (slate) |
| Neutral surface | `#0F1117` dark / `#FAFAF9` light |
| Success / Warning / Danger | `#22C55E` / `#F59E0B` / `#EF4444` |
| Type | Inter for UI, JetBrains Mono for code, tokens and numbers |
| Spacing scale | 4 / 8 / 12 / 16 / 24 / 32 px |
| Radius | 8px cards, 6px inputs, full for pills |

## 6. States

| State | Treatment |
|---|---|
| Index lagging | Tools include `stale: true` with commits behind. |
| Parse fallback | Line-window chunks flagged. |

## 7. Accessibility

- WCAG 2.2 AA contrast in both themes; never encode meaning with colour alone (pair with icon or label).
- Full keyboard navigation with visible focus rings; `/` focuses search, `?` opens shortcuts.
- Charts ship with a data-table fallback and `aria-describedby` summaries.
- Respect `prefers-reduced-motion`; streaming text is announced politely via `aria-live`.
