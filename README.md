# Omnifrons

**One context. Every model.**

Omnifrons is a planned cross-platform desktop facade for user-installed CLI agent harnesses. It gives a person one durable agent identity, one workspace, and one human-facing text/voice control surface while allowing the underlying model or harness to change.

> **Status:** development-mode spike. A Rust workspace plus a Tauri shell exist: a supervised harness runtime with typed IPC, executable approval, line and PTY adapters, and local publication and recovery surfaces. There is no supported release or installer.
>
> Formal verification against the desktop-stack verification plan (VP-001) has begun on a pinned Linux baseline. What that produced is filed in [`docs/evidence/VP-001/`](docs/evidence/VP-001/), failures included: the containment scenario's first completed run recorded a descendant that survived a stop the product had reported as clean, and that record is what drove the fix. The store is append-only, so a wrong result is corrected by a new record rather than an edit.

## Reading this repository

- **Build and test it:** [`docs/repository-layout.md`](docs/repository-layout.md) § Build and test commands. The short version is `cargo test --workspace` for the Rust side and `pnpm test` inside `renderer/`.
- **What is specified:** accepted capability specifications live in [`openspec/specs/`](openspec/specs/); the proposal, design, task and verification artifacts of each change live in `openspec/changes/`, archived by date once the change closes.
- **What is proven:** [`docs/evidence/VP-001/`](docs/evidence/VP-001/) holds the verification records and the procedures that produced them.
- **What was built, slice by slice:** [`docs/spike-log.md`](docs/spike-log.md).

## Product boundary

Omnifrons will discover, launch, supervise, and present harnesses that the user installs and authenticates independently. Candidate integrations include Claude Code, Codex, Gemini-oriented tooling, OpenCode, Pi, Qwen, GLM, Kimi, and provider-capable harnesses such as OpenRouter-configured clients. A name in the design is not a current compatibility promise.

Omnifrons is **not** a new model runtime, an autonomous-agent backend, a provider account broker, or a replacement for existing harnesses.

## Continuity model — target design, not yet implemented

Nothing in this section or the next exists in the code today. They describe where the
design is headed, and are kept here because they are what the spike is being built
towards; the Status box above is the account of what actually runs.

- The logical agent belongs to Omnifrons, not to a vendor session.
- Switching models uses an explicit checkpoint: summarize current work, add deterministic workspace evidence, stop the outgoing harness through its adapter, and start the next harness in the same project.
- Obsidian-compatible Markdown is canonical long-form knowledge and is always local. Its organization follows [Karpathy's LLM-Wiki pattern](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f), with [obsidian-skills](https://github.com/kepano/obsidian-skills) providing format and tool procedures.
- [Engram](https://github.com/Gentleman-Programming/engram) is curated operational memory and the first retrieval layer; Engram Cloud is optional.
- OpenSpec owns project specification and delivery artifacts.
- The Context Orb is a projection of those sources, never another source of truth.

## Storage model — target design, not yet implemented

Markdown and small portable state stay in the always-local Git tier. Heavy assets use a separate local or on-demand tier chosen by the user. Cloud-only assets remain visible as grey nodes in the Orb and become active only after an explicit, verified download.

The initial supported Linux blob path is the official Proton Drive CLI. User-managed `rsync` may coexist with Omnifrons but is outside the support and recovery contract.

## Documentation

- [Documentation index](docs/README.md)
- [Target architecture](docs/target-architecture.md)
- [Product roadmap](docs/roadmap.md)
- [Versioning and compatibility](docs/versioning-and-compatibility.md)
- [Context Orb presentation specification](docs/context-orb.md)
- [Architecture decisions](docs/adr/README.md)
- [Product naming and clearance](docs/product-naming.md)

## License

Licensed under the [Apache License 2.0](LICENSE). Third-party products and marks remain subject to their own terms. Omnifrons does not claim endorsement by the providers or projects named in its design.
