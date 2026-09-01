# Omnifrons Context Orb Presentation Specification

**Document role:** Context Orb and dashboard presentation — visual structure, theming, interaction, and dashboard widgets  
**Status:** Draft  
**Normative force:** Non-binding target direction; requirements are acceptance gates, not current guarantees  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** None

## Document authority

This document owns the presentation of the Context Orb and its surrounding dashboard: layer layout, node states, theming, interaction, and dashboard widgets. It does not own system invariants, storage tiers, or switching semantics — those belong to the [target architecture](target-architecture.md) — and it does not own maturity sequencing, which belongs to the [roadmap](roadmap.md). Where this document and the target architecture appear to conflict, the target architecture governs and the conflict blocks implementation of the disputed behavior.

## Definition

The **Context Orb** (short form: **Orb**) is the central visual of the Omnifrons dashboard. It is a read/status projection that shows, in one place: the active model/harness, available skills, operational memory, canonical knowledge context, scheduled routines, and connected applications.

The Orb inherits the projection invariants of the target architecture:

- It is a projection of authoritative sources (Markdown vault, Engram, OpenSpec, Context Catalog); it is never itself a persistence, synchronization, or conflict-resolution authority.
- A remote-only node becomes local only after explicit hydration and integrity verification.
- Pending, stale, forked, conflicted, uncertain, partial, and orphan-risk states are never shown as complete.

## Design lineage

The Orb synthesizes three established traditions: knowledge-graph visualization (interactive node maps over a document vault), command-center dashboards (peripheral widgets around a central visual), and classical geometry (hexagonal lattices, fractal recursion, the golden ratio). Every normative value in this document was fixed through the project's own design studies — the interactive geometry laboratory and the motion prototypes — and is owned by this specification.

## Visual structure

The Orb has two visual representations, fixed by the project's motion studies:

**Idle orb (dashboard center).** A particle globe: a cloud of 1–2 px colored dots, clustered by domain color, inside a faint geodesic wireframe sphere with scattered star specks. Idle motion is slow rotation (~1 revolution per ≥8 s) plus per-particle twinkle — there is no measurable scale or position "breathing". At the sphere's heart orbits a **cube of sub-cubes** rotating about its own space diagonal, so its silhouette remains hexagonal: a hexagon is a cube seen along that diagonal, which makes the idle orb the 3-D lift of the hexaflake graph — the cube that is conceptually a sphere (the squaring of the circle). A dot-matrix CTA beneath the globe invites opening the full view.

**Interactive graph view (takeover surface).** Activating the idle orb plays a three-beat transition — particle implosion (~0.5 s, ease-in), hard white flash (~0.1 s), ease-out fade (~0.4 s) — into a full-screen concentric graph on the warm near-black ground, textured by the hexagonal mesh, which **continues past the applications band and fades out with radius** — the modeled region dissolves into space rather than ending at a border. From the center outward:

| Ring | Name | Content | Color rule |
| --- | --- | --- | --- |
| Core | Context root | Rounded-square node bearing the active-model glyph, wrapped in a bright ring with a soft glow; the vault's root index, labeled below in caps | Active-model brand color |
| 1 | SKILLS | Thin guide ring carrying its zone label set on the ring; skill nodes render as vertical rhombi along it | Active-model brand color |
| 2 | DOMAINS | Filled circles of equal size with a thin lighter ring, soft glow, and a white line glyph, placed outside the SKILLS ring at irregular angles; labels below in caps (see Domain lifecycle) | One distinct color per domain, assigned at declaration and recorded in the root index |
| 3 | MEMORY | Concentric arcs of same-color dots per domain sector, dot size growing outward; the arc extent marks the MEMORY boundary | Dots inherit the domain color; connection lines are curved (~1–1.5 px, opaque) and take the child node's color |
| 4 | ROUTINES | Thin zone ring with orbit-disc nodes — a filled disc inside a thin orbit ring with a satellite dot riding it, the glyph for "a scheduled thing that goes round" | Amber-family zone ring and markers |
| 5 | APPLICATIONS | Connected external capability: human-launched applications and micro-apps, plus agent-facing connectors (MCP servers, plugins, extensions) | Blue-family zone ring; application nodes snap to cells of the background honeycomb and render as that cell lit up (glyph, glow, translucent fill); the two node natures carry distinct visual treatments |

### Geometric harmonization (validated: hexaflake lattice)

Chosen from the five-candidate geometry study and validated interactively in its "Hexaflake orb" mode: the definitive Orb derives its layout from a **hexaflake fractal** — each hexagon spawns seven copies at 1/3 scale. The fractal governs radii and positions; **φ governs the size steps** (node radii step by φ² per importance level — domain : topic : dust ≈ 11 : 4.2 : 1.7), because the flake's ÷3 scale factor and a continuous φ radius progression are incompatible (~14.6% per level).

Validated anchor mapping:

- **Core** occupies the flake's center cell and bears the **active model's mark** in its brand color; the brand color also drives the SKILLS ring and the skill rhombi. Prototypes use monograms as stand-ins for the marks.
- **Skills** sit on the center cell's boundary vertices (S/3) — the core's immediate surface.
- **Domains** occupy the six level-1 cell centers (2S/3) — a natural cap of six per view level, free slots rendered as dashed anchors. Domain colors are assigned in **color-wheel order** around the ring (yellow → orange → red → violet → blue → green), so adjacent territories are natural wheel neighbors and cross-territory bond gradients blend rather than clash.
- **Memory is territorial**: each domain's topics render as its level-2 sub-cells, keeping **five topics per territory** — the outward anchor chain belongs to the domain's routine, never to memory. Notes and dust fill the deeper levels.
- **Routines** anchor on the outward level-2→3 chain at 26S/27; **applications** sit just beyond the flake's outer edge (≈1.1S) — external capability living literally outside the knowledge boundary. The bands stay disjoint with clear air: memory ≤ ~0.8S, routines 0.963S, applications 1.1S.
- **Node geometry is semantic, never decorative**: shape encodes layer — hexagons for applications, orbit-discs for routines, vertical rhombi for skills, plain spheres for notes, and a **ringed sphere with a count badge** for any collapsed aggregate.
- Connection lines still terminate on **vertices**, never centers, per the registered vertex rule.
- **Concentric zone guide rings** (skills S/3, domains 2S/3, routines 26S/27, applications ≈1.1S) stay visible over the flake; memory deliberately has no ring — it is territorial.
- **Whisper budget**: the flake mesh renders as background texture (opacity fading by level), and cross-domain relations are an **invisible attraction** — no permanent strokes. A relation materializes as a single gradient hairline only while its topic (or domain) is hovered or dragged, with opacity encoding strength, and while auto-arrange is pulling; at rest, relation strength lives purely in proximity. The whisper budget is global — multiplying elements divides their per-element presence.

Interaction, validated in the study:

- **Obsidian-style dragging**: dragging a domain carries its whole territory; dragging a topic moves it alone with its dust and bonds. Bond topology derives from base geometry and stays fixed while dragging.
- **Auto-arrange by relations**: a bounded force pass in which each bond pulls its endpoints proportionally to its order (triple strongest) while every displacement decays back toward its fractal anchor — relations bend the layout, the lattice reclaims it.
- **Fractal inner worlds ("as above, so below")**: activating a domain dives one level down with a zoom transition into that domain's inner world — its index at the center, its topics as inner domains, their notes as territories, inner bonds between them — rendered monochrome in the domain's color. The inner world is equally draggable; surfacing returns to the outer view. Navigating the vault hierarchy (root index → domain index → topic index, the LLM-Wiki recursion) is literally zooming the fractal.
- **Radial context menu (target)**: activating a node can raise a ring menu around it — expand, collapse, hide, pin — without disturbing the rest of the graph; pinned nodes hold position through auto-arrange, and saved partial views capture a focus arrangement for later.

The territorial memory model is validated and supersedes ring 3's concentric arcs; the ring table above is retained as the earlier annular model this design evolved from.

### Aggregation and level of detail

The Orb scales by aggregation, never by drawing everything:

- A collapsed folder or sub-index renders as a single ringed sphere carrying a **count badge**; expansion is explicit and per-node. On expansion, children emerge from the parent and travel outward along curved links with an ease-out settle (~1–1.5 s), and a confirmation toast names the result ("Expanded ⟨name⟩ · N items"). A second activation collapses the aggregate. Click and drag are disambiguated by a small movement threshold, so aggregates stay draggable like any node. Bulk expand/collapse operations complement per-node control. Validated in the geometry study: each domain territory carries one aggregate standing in for its deepest sub-index.
- Bubble radii are graduated (roughly a 6:1 range, switchable to a log scale for power-law corpora) and **numeric badges appear only above a size threshold** — a size-based label budget. Note labels are a global toggle, not a zoom behavior; layer and domain labels live in world space.
- Out-of-focus branches desaturate toward the background while the active branch keeps full color.
- The formal model is a **multi-resolution cluster index**: badge counts and aggregate metrics are precomputed in the store, a resolution cut marks which aggregates are collapsed or expanded, and expansion is a local operation — never a full-graph traversal.
- Which nodes surface at each zoom level follows an importance ranking (top ⌈|V|/2ᵏ⌉ by centrality per level, scaled to avoid collisions). Zooming preserves **monotone nesting** — nothing visible ever disappears by zooming in — and newly revealed nodes emerge in the **angular gaps around their persistently positioned parent**, preserving the mental map.

### Scale rules (semantic zoom)

- **Macro — the orb**: domain hubs and aggregates with accumulated badges; only top-ranked nodes surface; relations stay invisible attraction.
- **Meso — a territory**: aggregates unfold into sub-clusters; bridge nodes (high betweenness centrality) surface; revealed hairlines may bundle.
- **Micro — an inner world**: full atomic expansion — complete labels, 1–2-hop neighborhood, content previews.

### Rendering budget (definitive build)

SVG holds to roughly a thousand elements and Canvas to ~5k; the 10–35k-node target requires **WebGL**. Decision: **Sigma.js v3 + Graphology** — WebGL drawing fed exact, deterministically computed coordinates (the hexaflake anchors), layout physics confined to WebWorkers, and custom shaders for the non-circular glyphs. GPU engines that own the layout through free-running force grids are rejected: they cannot honor a deterministic orbital lattice. Prototypes remain SVG/Canvas within their element budgets.

### Relation intelligence

Community detection (modularity) plus betweenness centrality identify dense clusters that barely talk to each other; the agent surfaces these structural gaps as **bridge proposals** — candidate cross-domain relations, and the questions that would connect them — through the standard proposal-and-approval flow, never auto-created.

### Deterministic projection feed

The Orb's data — and any memory retrieval behind it — is produced by plain deterministic code before any model is involved:

1. Keyword extraction from the query, filler discarded.
2. Index-based **scoring of candidate notes without opening them**, against the vault's indexes and reference maps.
3. Open only the top-scored note; extract the specific answering section, never the whole file.
4. If the section is a pointer, follow it — deterministically and bounded.
5. Hand only the extract to the model, which is invoked once.

For the operational-memory plane this feed **is Engram**: its search runs as deterministic code over the local store and returns curated, scored observations whose pointers lead into the vault — exactly steps 2–4, already implemented. The vault side adds index/reference-map scoring over canonical notes (the LLM-Wiki index layer); the two stages compose — observation hit → pointer → vault note → section extract — and neither involves a model call.

Feed scoring upgrades, all composing over the SQLite substrate:

- Hybrid keyword + vector fusion via **adaptive reciprocal-rank fusion**: query-IDF-driven weighting (rare exact terms boost keyword search; diffuse queries boost vectors) with a distance-based relevance cutoff.
- **Structure-aware tokenization**: compound identifiers split for recall without losing exact-match precision.
- A **single SQL round-trip** combining full-text search, vector similarity, and bounded recursive-CTE graph traversal, with ontology closure precomputed at insert time so hierarchy expansion is O(1) at query time.
- **Bitemporal pointer edges** (valid-from / valid-until) so the feed reads current pointer state — or any past state — without parsing markdown on the hot path.
- Anti-pattern, evidence-backed: **post-fusion temporal decay degrades retrieval precision and is excluded**.

This is the concrete mechanism behind the "typed projection data" invariant and the product's token economy. The acceptance methodology is a paired-session benchmark — the same prompt against default retrieval and against the index feed — with per-category context accounting; the target range is 40–60% savings in both tokens and wall time, with answer correctness verified on every run.

Graph motion and control: the idle graph is near-static — ring rotation is a user parameter ranging from static to a slow continuous drift, and only sub-pixel drift plus twinkle animates while a selection is held. Selecting a node re-renders its branch in a single highlight color with curved lines to its children while the rest dims; selecting a domain fans out its file nodes and desaturates everything else (filter mode). Idle lines carry domain colors; selection overrides them with the highlight color. The layout is switchable (Force / Circle / Hex / Rings — Rings is the default), grouping toggles between domains and folders, and graph parameters (ring spin, link springs, node size, label visibility, expand/collapse) live in the view's menu panel. File labels appear only near zoom or on hover; ring and domain labels survive zoom-out.

Node availability states, in every ring:

- **Local:** rendered in the ring's active color.
- **Remote-only:** rendered grey, per ADR-0003. Opening one starts an explicit download; the node changes to local only after integrity verification.
- **Harness-unavailable:** capabilities the active harness cannot provide are rendered dimmed/desaturated with an explicit disabled affordance, and read-only where their definition can still be inspected (for example, Claude-native local routines while a non-Claude harness is active). This state is visually distinct from remote-only grey because the remedy differs: a remote-only node needs hydration; a harness-unavailable node needs a different active harness. Availability is resolved per capability from the active adapter's capability probe, never from a hardcoded per-model feature list. Nodes in this state are never hidden as if absent and never shown as active.

Ring 5 hosts two node natures with distinct visual treatments: human-launched applications (opened by the user) and agent-facing connectors (MCP servers, plugins, extensions used by a harness). A connector is not a new ring: it is the same ontological category — connected external capability — as any application node. Connector availability follows the same per-capability probe and three-state rendering as every other node, so a connector lights up under any harness whose adapter can reach it and dims under one that cannot. When ring 5 grows beyond comfortable density, nodes cluster by category (media, advertising, communication, data, …) and expand in inspect mode; the ring never renders an unbounded flat list.

Ring 2 domains are user-specific data governed by the domain lifecycle below; this document deliberately ships no example domain list.

## Domain lifecycle

The ring 2 domain list is data owned by the knowledge plane, never presentation configuration:

- **Declaration authority.** A domain exists when the vault's root index (the LLM-Wiki router layer) declares it. The Orb renders the declared list and never derives, invents, or hardcodes domains at the presentation layer (invariant 10).
- **Membership authority.** A note belongs to a domain because it lives under the domain's folder and its domain index links it. Folders plus index links are the classification mechanism. Tags remain secondary metadata for search and filtering and never determine ring membership. This keeps membership deterministic (one note, one branch), Git-diffable, and readable by any harness.
- **Emergence with curation.** No domain is created silently. The vault maintenance routine proposes a new domain when unfiled content accumulates that fits no declared domain; the user approves, renames, or rejects the proposal (invariant 1). On approval the routine updates the root index, including the domain's assigned color, so colors stay stable across renders. The user may also declare a domain manually at any time.
- **No shipped taxonomy.** Omnifrons ships no predefined domain list. Onboarding proposes an initial set instead: for an existing vault, a small set derived from a content scan; for an empty vault, either no domains (inbox only) or a minimal seed proposed from the user's stated intended use. Proposals become domains only on user approval.
- **Empty and unfiled states.** A declared domain with no content is not rendered (at most as a dimmed placeholder). Content not yet classified lives in the raw-sources/inbox zone and is surfaced as an explicit unfiled indicator on the Orb, never hidden and never shown as filed (invariant 8).

## Active model presentation and theming

The dashboard always shows which model/harness is active. The default is Claude Code; candidate integrations follow the product boundary in the [README](../README.md).

- The root element carries a `data-model` attribute; changing it swaps the model brand color instantly at the presentation layer (base theme and user accent are separate; see Style system). Reference brand colors: Claude `#D97706`, ChatGPT `#10B981`, Codex `#38BDF8`, Qwen `#8B5CF6`, Gemini `#2563EB`. Additional harnesses (GLM, Kimi, OpenRouter-configured clients, …) receive theme entries when their adapters exist.
- Switching is requested from a visible selector on the dashboard or a `/switch <model>` command. Either path triggers the checkpoint-and-restart handoff defined in the target architecture; only the visual theme change is instantaneous. The UI must not present a switch as complete while the handoff is `uncertain`.
- Truthful capability status: harness-native features degrade visibly on switch. For example, Claude-native local routines become disabled/read-only while a non-Claude harness is active; cloud-scheduled routines remain active through the adapter of the selected harness.
- The logical agent's identity and personality do not change with the model. They are supplied by Omnifrons-owned context, not by any vendor session.
- Active does not mean exclusive. Exactly one model is the foreground interactive harness — it drives the conversation surface and the `data-model` theming — while other harnesses may execute concurrently: cloud-scheduled routines, headless skill runs, and background tasks. Concurrent executions are surfaced by the sessions monitor widget with their own harness identity and never inherit the foreground theming. Execution concurrency never weakens the single-writer coordination of portable state defined by the target architecture.

## Style system

Warm-industrial command-center aesthetic, fixed by the project's design studies.

### Base tokens

- **Dark theme (default):** background `#131311`, card `#1C1A16`, text `#E8E2D2` (bone), muted `#8D8775`, shadow `rgba(28, 26, 22, 0.85)`.
- **Light theme (paper):** background `#F2EFE8`, card `#FAF8F2`, text `#1C1A16`, muted `#6F6A5C`, shadow `rgba(232, 226, 210, 0.30)`.
- **Status colors:** success `#10B981`, attention `#F59E0B`.
- **Shape:** 12–16 px radius on primary containers, 6–8 px on inner controls; subtle depth shadows; restrained glow only on interactive elements.
- **Industrial details:** two-digit step numerals (`01`, `02`, …) set in the dot-matrix face; hexagonal SVG cursors on interactive zones; pixel-art avatar set for agent identities.

### Typography

- **Doto** (dot-matrix variable face; weights 400/600/700/900) for display identity: wordmark, panel tab labels, zone labels, and large numerals. Labels render uppercase with wide tracking (1–6 px).
- **Outfit** (geometric sans; weights 100–900) for UI text and reading content, with tight negative tracking (−0.2 to −1.5 px) on headings.
- A monospaced face remains for machine values: identifiers, model names, effort levels, schedule times, and spend counters.

### Accent themes (user preference)

The dashboard chrome carries one user-selected accent, independent of the active model's brand color:

- **Industrial Orange `#FF6B1A`** — default; it reads coherently with Claude as the default model brand.
- **Volt Yellow** — `#FFF000` or a nearby more fluorescent variant; the exact value is pending per-theme contrast validation, especially on the light/paper theme.
- Additional curated accents are an open decision; the set stays deliberately small.

### Two theming layers

Base theme (dark/light) and accent are user preferences applied to the dashboard chrome. The `data-model` brand color (see Active model presentation and theming) overlays only model-bound elements: the core, the SKILLS ring, routine markers, and the model selector. Changing the model never changes the user's base theme or accent; changing the accent never obscures which model is active.

Widgets around the Orb sit on a flexible grid with 16–24 px gaps and support drag-and-drop repositioning and resizing.

## Interaction model

- **Idle orb:** slow rotation with per-particle twinkle; the perceived "breathing" is rotation plus a nucleus brightness oscillation, never a scale pulse.
- **Opening transition:** particle implosion (~0.5 s, ease-in) → hard white flash (~0.1 s) → ease-out fade (~0.4 s) into the full-screen graph view; roughly 1 s total.
- **Hover:** a tooltip pins beside the node with its name, counts ("N files · N md · N KB"), path, the state verb ("click to expand" / "click to collapse"), and scope pills.
- **Selection:** the branch re-renders in the highlight color and everything else dims; a domain click fans out its files ("click to filter"). Selection opens the **entity card** — a compact panel with title, chips (domain plus executing-agent/permission chips), metadata, path, a canonical action row (fly to, edit, view here, open on device / open folder, copy path, expand, remove — destructive always last and red-tinted) and a connections list. The hover tooltip is the short form of the same record, ending with its affordance hint ("click to expand"). "View here" (file nodes only) opens the **reader panel**: an in-app dock over roughly a quarter to 40% of the view rendering the file's markdown, with code blocks scrolling horizontally in place.
- "Open on device" / "open folder" are external launches governed by the surfaces model and the renderer content-security contract.
- **View chrome:** a "back to the OS" pill returns to the dashboard; a menu pill opens the panel with global search ("/" shortcut) and the graph parameters above.
- Opening a remote-only node is the explicit hydration flow of the target architecture, with progress and verification state visible; the node is not recolored until verification succeeds.
- The Orb renders only typed projection data supplied through the application layer. It never renders raw content from synchronized sources; rich rendering remains subject to the planned renderer content-security contract (RCS-001).

## Dashboard surfaces

Every view reachable from the dashboard uses one of four surface patterns. All of them run inside the untrusted renderer and inherit the renderer content-security constraints (RCS-001 once accepted; plain/constrained defaults until then).

| Surface | Behavior | Examples |
| --- | --- | --- |
| In-place widget | Expands and collapses within its own grid cell; no navigation | Calendar, email summary, time zones, usage and spend widgets |
| Takeover view | Expands to a full-viewport view inside the same renderer; no new OS window | The Orb map opened from the core; inspect-mode detail |
| Tabbed panel | Routes between dashboard panels inside the dashboard container | Routines detail (schedule rules plus generated output drafts), skills deck expanded view, artifacts search, headless-run result reports |
| External launch | Leaves the dashboard: opens an artifact, micro-app, or application in an external surface | Opening a generated HTML artifact, micro-app quick links, application deep links |

Rules:

- External launches are navigation actions governed by the renderer content-security contract: constrained URL providers/schemes and explicit user confirmation. The dashboard never auto-launches an external surface as a side effect of rendering.
- A headless skill run fired from the skills deck completes into a result report presented as a tabbed panel or artifact reference; execution feedback is never delivered by silently opening external content.
- Surface choice is presentation policy and never changes authority: whatever surface displays content, the Orb and dashboard remain read/status projections (invariant 10).

## Dashboard widget roster

Peripheral widgets form the modular surface around the Orb. This roster is target direction, not a commitment; every widget obeys the surfaces model above and the projection invariants.

Adapted from prior art:

- **Skills deck:** select and fire skills with effort level and model selector; runs complete into result reports.
- **Artifacts search:** history and filter over generated artifacts.
- **Daily summary:** calendar, time zones, and flagged mail.
- **Routines board:** scheduled routines with next-fire times.
- **Micro-app quick links** and user-defined custom widgets.

Omnifrons-native (no prior-art equivalent):

- **Checkpoint/handoff:** current checkpoint state, published/claimable handoffs per device, and switch progress — an `uncertain` handoff is always shown as such, never masked.
- **Approval queue:** every pending human decision in one place — domain proposals, executable approvals, itemized elevations (invariant 1 made visible).
- **Sync health:** authority status per data plane — Engram chunk state and pending imports, workspace roaming state, authority conflicts, and memory freshness (time since the last curated memory save). The freshness nudge — prompting the agent to persist significant work after a configurable interval — is an Omnifrons MemoryCoordinator policy applied through every harness adapter, never a dependency on any single harness's plugin hooks. It reminds; the agent saves; it never writes memories itself.
- **Sessions monitor:** every running harness execution — foreground session, headless runs, cloud routines — with harness identity and state.
- **Generations gallery:** log of AI-generated media fed by agent connectors.
- **Scope/trust indicator:** the active harness's scope mode (`sandbox-enforced`, `harness-enforced`, `advisory`) permanently visible.

## Dashboard usage and spend widgets

The dashboard includes a consumption module with two distinct visualizations, because the two billing models measure different things:

1. **Subscription quota bars.** For subscription plans (Claude, ChatGPT, Gemini, Qwen, GLM, Kimi, …): a progress-style bar per plan showing consumed versus available quota for the current billing window, with the reset time. Bars use the plan's brand color and the standard status colors near exhaustion.
2. **Metered spend counters.** For pay-per-token providers (OpenRouter, DeepSeek, …): cumulative token and cost counters for a selected window, rendered as monospaced numeric tiles with optional trend sparklines. No artificial progress bar is drawn where no quota exists.

Constraints:

- Omnifrons is not a provider account broker. Usage data may come only from user-authorized, adapter-visible surfaces (local harness telemetry, provider usage APIs the user has configured, or user-supplied exports). Provider credentials remain harness- or user-owned per the target architecture's secret rules.
- Unknown or stale usage data is shown as unknown/stale, never as a fresh zero.
- The concrete data source per provider is an open implementation decision and may land after the widget's visual contract.

## Open decisions

| Topic | Status |
| --- | --- |
| Onboarding domain-proposal flow (content-scan heuristics, intended-use questions, proposal UX) | Open implementation decision; constrained by the Domain lifecycle section |
| Theme entries for GLM, Kimi, OpenRouter-based harnesses | Open; pending adapter existence |
| Accent theme set beyond Industrial Orange and Volt Yellow; final Volt Yellow value | Open; requires per-theme contrast validation |
| Connector category taxonomy for ring 5 clustering | Open; derive from the connectors users actually attach |
| Model marks at the core: licensed logos vs designed monograms | Open; prototypes use monograms |
| Domain overflow beyond the six-slot wheel (paging, nesting, or merge) | Open |
| Inner-world recursion beyond one level (topic worlds) | Open; one level validated in the study |
| Usage-data source per provider (API, local telemetry, manual) | Open implementation decision |
| Orb data feed implementation (index formats, scoring function, pointer-follow bounds for the deterministic projection feed) | Open; the feed's shape is specified in "Deterministic projection feed" |

## References

- [Target architecture](target-architecture.md) — invariants 10–12, knowledge/memory/delivery planes, switching semantics, secret rules.
- [ADR-0003](adr/0003-local-markdown-and-tiered-assets.md) — local Markdown, tiered assets, grey remote-only nodes.
