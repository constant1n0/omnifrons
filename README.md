# Omnifrons

**One context. Every model.**

Omnifrons is a planned cross-platform desktop facade for user-installed CLI agent harnesses. It gives a person one durable agent identity, one workspace, and one human-facing text/voice control surface while allowing the underlying model or harness to change.

> **Status:** design phase. This repository does not yet contain a working application, installer, or supported release.

## Product boundary

Omnifrons will discover, launch, supervise, and present harnesses that the user installs and authenticates independently. Candidate integrations include Claude Code, Codex, Gemini-oriented tooling, OpenCode, Pi, Qwen, GLM, Kimi, and provider-capable harnesses such as OpenRouter-configured clients. A name in the design is not a current compatibility promise.

Omnifrons is **not** a new model runtime, an autonomous-agent backend, a provider account broker, or a replacement for existing harnesses.

## Continuity model

- The logical agent belongs to Omnifrons, not to a vendor session.
- Switching models uses an explicit checkpoint: summarize current work, add deterministic workspace evidence, stop the outgoing harness through its adapter, and start the next harness in the same project.
- Obsidian-compatible Markdown is canonical long-form knowledge and is always local. Its organization follows [Karpathy's LLM-Wiki pattern](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f), with [obsidian-skills](https://github.com/kepano/obsidian-skills) providing format and tool procedures.
- [Engram](https://github.com/Gentleman-Programming/engram) is curated operational memory and the first retrieval layer; Engram Cloud is optional.
- OpenSpec owns project specification and delivery artifacts.
- The Context Orb is a projection of those sources, never another source of truth.

## Storage model

Markdown and small portable state stay in the always-local Git tier. Heavy assets use a separate local or on-demand tier chosen by the user. Cloud-only assets remain visible as grey nodes in the Orb and become active only after an explicit, verified download.

The initial supported Linux blob path is the official Proton Drive CLI. User-managed `rsync` may coexist with Omnifrons but is outside the support and recovery contract.

## Documentation

- [Documentation index](docs/README.md)
- [Target architecture](docs/target-architecture.md)
- [Product roadmap](docs/roadmap.md)
- [Versioning and compatibility](docs/versioning-and-compatibility.md)
- [Architecture decisions](docs/adr/README.md)
- [Product naming and clearance](docs/product-naming.md)

## License

Licensed under the [Apache License 2.0](LICENSE). Third-party products and marks remain subject to their own terms. Omnifrons does not claim endorsement by the providers or projects named in its design.
