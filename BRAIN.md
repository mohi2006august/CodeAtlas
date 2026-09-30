# CodeAtlas: Brain

> Second brain for this project: concepts, mental models, research, ideas and decisions. Keep it messy and alive.

## 1. Why This Project

It sits at the intersection of retrieval, representation learning, tokenization cost, context engineering and agent protocols, applied to the domain where agents are most used today: code.

## 2. Concept Primers

### MCP

Model Context Protocol: an open JSON-RPC protocol that lets AI clients (Claude, IDEs, agents) discover and call tools, read resources and use prompts exposed by servers in a standard way.

- **Server**: Exposes tools, resources and prompts over stdio or Streamable HTTP.
- **Tool**: A typed function the model can call (a JSON Schema input and a structured result).
- **Resource**: Read-only context addressed by URI (files, rows, docs) that the client can attach.
- **Transport / auth**: stdio for local servers, Streamable HTTP + OAuth 2.1 for remote servers.

> **Mental model:** Think of MCP as USB-C for AI. Write the integration once and every compliant client can use it. Tool descriptions are prompts, so write them for the model.

### RAG

Retrieval-Augmented Generation: fetch relevant passages from an external corpus at query time and put them in the prompt so the LLM answers from evidence instead of memory.

- **Chunking**: Splitting documents into retrievable units (fixed tokens, sentence windows, semantic or structure-aware splits).
- **Hybrid retrieval**: Combining sparse (BM25) and dense (embedding) search, usually merged with Reciprocal Rank Fusion.
- **Re-ranking**: A cross-encoder rescoring the top-k candidates for precision before they reach the prompt.
- **Grounding / faithfulness**: Whether every claim in the answer is supported by a retrieved passage.

> **Mental model:** Retrieval sets the ceiling and generation sets the floor. If the right chunk never gets retrieved, no prompt can save the answer, so measure recall@k before tuning prompts.

### Embeddings

Dense vector representations where geometric closeness reflects semantic similarity; the backbone of search, clustering, recommendation and RAG.

- **Contrastive training**: Pulling positive pairs together and pushing negatives apart (e.g. InfoNCE).
- **Matryoshka embeddings**: Vectors whose prefixes are themselves usable lower-dimensional embeddings.
- **Anisotropy**: Embeddings crowding into a narrow cone, which hurts cosine similarity spread.
- **Embedding drift**: Distribution shift in vectors over time or after a model upgrade.

> **Mental model:** An embedding model is an opinion about what 'similar' means. Benchmarks like MTEB are a starting point, so evaluate on your own queries.

### Tokenization

Converting raw text into the integer token IDs a model consumes. Subword algorithms (BPE, WordPiece, Unigram) balance vocabulary size against sequence length.

- **BPE**: Byte-Pair Encoding: iteratively merges the most frequent adjacent pair; byte-level BPE never hits unknown tokens.
- **Unigram LM**: Starts with a large vocabulary and prunes tokens that least reduce corpus likelihood (SentencePiece).
- **Fertility**: Average tokens per word; high fertility means higher cost and less effective context for that language.
- **Special tokens**: BOS/EOS/PAD and chat-template markers that structure model input.

> **Mental model:** Tokens are the unit of cost, latency and context. The same sentence can cost 3x more in Hindi than in English, and that 'token tax' is a product and fairness issue.

### Context Window

The maximum number of tokens (prompt plus output) a model can attend to in one call. Everything the model 'knows' about the task at inference time must fit inside it.

- **Lost in the middle**: Models attend less reliably to information buried mid-context than at the start or end.
- **Compaction**: Summarising or pruning older turns so long sessions fit the budget.
- **KV cache**: Stored attention keys/values; grows linearly with context length and dominates memory.
- **Needle-in-a-haystack**: Eval that plants a fact at varying depths to test long-context recall.

> **Mental model:** Context is a budget, not a bucket. More tokens cost more money and latency and can dilute attention, so curate what goes in.

### Caches

Storing results of expensive operations for reuse. In LLM systems this means exact response caches, semantic caches, embedding caches, and provider-side prompt/KV caching.

- **Hit ratio**: Fraction of requests served from cache; the core ROI metric.
- **Semantic cache**: Returns a cached answer if a new query is similar enough (embedding distance < threshold).
- **Prompt caching**: Provider reuses computed KV for a repeated prompt prefix, cutting cost and latency.
- **Invalidation / TTL**: When cached answers become stale; famously one of the two hard problems.

> **Mental model:** A cache is a bet that the future looks like the past. A wrong semantic cache hit costs more than a miss, so tune thresholds for precision.

### Fine Tuning

Continuing training of a pre-trained model on task- or domain-specific data to change its behaviour, style or knowledge.

- **SFT**: Supervised fine-tuning on prompt → ideal response pairs.
- **DPO / preference tuning**: Optimising directly on chosen-vs-rejected pairs without a separate reward model.
- **Catastrophic forgetting**: Losing general capabilities while specialising.
- **PEFT**: Parameter-efficient methods (LoRA, adapters, prefix tuning) that train a small fraction of weights.

> **Mental model:** Fine-tune for form (format, tone, task behaviour). Retrieve for facts. Data quality beats data quantity almost every time.

### Vector Database

A datastore optimised for storing high-dimensional vectors and answering approximate nearest-neighbour (ANN) queries, usually with metadata filtering.

- **ANN index**: HNSW (graph), IVF (clustered lists), PQ (compressed codes), DiskANN (SSD-resident graph).
- **Recall@k**: Fraction of true k nearest neighbours returned; the quality side of the speed trade-off.
- **Filtered search**: Applying metadata predicates pre-, post- or in-graph during traversal.
- **Distance metric**: Cosine, dot product or L2; must match how the embeddings were trained.

> **Mental model:** Every ANN index trades recall, latency, memory and build time against each other. Pick two to optimise and measure the other two.

## 3. What I Want to Learn

- [ ] Static analysis with tree-sitter.
- [ ] Graph-augmented retrieval.
- [ ] Mining training data from Git history.
- [ ] Token-budgeted context rendering for code.

## 4. Reading & Research List

- Jimenez et al., *SWE-bench* (2023)
- Zhang et al., *RepoCoder* (2023)
- Sourcegraph, *Cody context retrieval* engineering posts
- Husain et al., *CodeSearchNet* (2019)
- tree-sitter documentation

## 5. Experiments to Run

| # | Hypothesis | How to test |
|---|---|---|
| 1 | Repo-fine-tuned embedder +10 recall@10 vs base | Time-split benchmark. |
| 2 | Signature-only far context keeps success with 60% fewer tokens | Agent A/B. |
| 3 | Graph expansion finds tests missed by pure search | Measure tests-for recall. |

## 6. Ideas Backlog

- LSP-backed precise references.
- Architecture map resource (auto-generated module overview).
- Ownership/CODEOWNERS-aware answers.

## 7. Decision Log

| Date | Decision | Reason |
|---|---|---|
| YYYY-MM-DD | Symbol-level chunks | Line windows split functions and hurt recall. |
| YYYY-MM-DD | Time-split evaluation | Random split overstated gains. |

## 8. Scratchpad

- _Dump raw thoughts, links and half-formed ideas here._
