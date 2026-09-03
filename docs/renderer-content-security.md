# Renderer Content-Security Contract

**Document role:** Renderer content-security contract: content classes, sanitization, CSP, navigation, terminal control policy, clipboard, attachments and downloads, redacted exports (RCS-001)  
**Status:** Draft  
**Normative force:** Non-binding target direction; requirements are acceptance gates, not current guarantees  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Document authority

This document drafts the planned renderer content-security contract (RCS-001): content classes and their rendering modes, sanitization allowlists, the CSP baseline, navigation and URL constraints including the external-launch confirmation, the terminal control and OSC policy that AEC-001's normalization enforces, clipboard policy, attachments and downloads, images and remote resources, the redaction and safe-serialization boundary for scrollback and support exports, the constraints third-party apps inherit, and the acceptance evidence for the renderer security gate. The [target architecture](target-architecture.md) governs any conflict.

This draft does not redefine work already owned elsewhere:

- **AEC-001** owns the PTY normalization implementation and typed terminal actions — the transport that turns allowlisted raw bytes into typed actions (drafted in part by [adapter-feed-events.md](adapter-feed-events.md)). RCS-001 states the policy that normalization must enforce; it does not restate AEC-001's mechanism.
- **[TM-001](threat-model.md)** owns the attacker model — actor classes, trust boundaries, and residual risk. RCS-001 enforces the boundary TM-001 states as B1 (renderer ↔ core) and supplies the mitigation TM-001's CNT-1..6, HAR-5, and PRC-2 threats credit.
- **AEC-001** (feed profile) owns the approvals write path; **[TM-001](threat-model.md)-R7, HAR-6, and INJ-3** own what an approval surface must display. RCS-001 does not restate either requirement — it guarantees only that no rendered content can appear inside an approval surface, and that no terminal pane hosts one, as if it were part of it.
- **[context-orb.md](context-orb.md)** owns presentation and layout — how a reader panel, terminal frame, or entity card is composed on screen. RCS-001 owns what content class each surface is permitted to hold and what that content is permitted to do.
- **VOC-001** owns voice interaction and accessibility text fallback generally. RCS-001 guarantees only that sanitization never removes textual content or reading order.
- **UTA-001** owns update trust for the product's own binary; unrelated to rendered content.
- **[ADR-0002](adr/0002-desktop-technology-stack.md)** owns the desktop stack decision. RCS-001 is the "renderer-content review" its acceptance evidence (#2) names.
- **[ADR-0004](adr/0004-open-platform-and-custom-apps.md)** owns the third-party app distribution and monetization decision. RCS-001 states the content-security constraints a custom app must respect (ADR-0004, Costs and limits).
- **RSP-001** owns credential custody procedures generally, including the Engram Cloud token's device-local storage. RCS-001 only guarantees that a custody-owned secret never appears inside a rendered surface or an export this document serializes.

## Purpose and scope

RCS-001 governs the **renderer content-security boundary**: what the untrusted renderer is permitted to render, request, and reveal, and what it can never do without a typed IPC request crossing to core and, where it matters, a device-local approval. It is the acceptance evidence the roadmap names for the Alpha renderer content-security baseline and the Alpha → Beta renderer-content review, and the artifact TM-001, the target architecture, and ADR-0002 all point to instead of restating renderer mechanics themselves.

In scope:

- content classes and their default rendering modes: plain text, rich (Markdown), terminal (PTY output), attachments/downloads, feed and memory content;
- sanitization allowlists and what is always stripped;
- the terminal control and OSC policy AEC-001's normalization must enforce;
- navigation, URL scheme constraints, and the external-launch confirmation;
- images and remote resources, and why the renderer never fetches them itself;
- clipboard read/write policy;
- the CSP baseline enforced on every renderer surface;
- typed IPC constraints on every renderer→core request;
- the redaction and safe-serialization boundary for scrollback and support exports;
- the constraints a third-party app inherits from this contract with no relaxation;
- the acceptance evidence that satisfies the renderer content-security gate.

Out of scope:

- the PTY normalization implementation and typed terminal action catalog — AEC-001;
- the attacker model, trust boundaries, and residual risk — TM-001;
- the approvals write path and what an approval surface must display — AEC-001; TM-001-R7;
- presentation and layout of any surface this document constrains — [context-orb.md](context-orb.md);
- voice interaction and accessibility text fallback generally — VOC-001;
- update trust for the product's own binary — UTA-001;
- the desktop stack decision itself — [ADR-0002](adr/0002-desktop-technology-stack.md).

RCS-001's guarantees are independent of the harness scope mode (`sandbox-enforced`, `harness-enforced`, `advisory`) the target architecture defines: the renderer boundary and the harness containment boundary are two different trust boundaries — B1 and B2, per TM-001 — and a harness running under `advisory` scope changes nothing about what the renderer itself is permitted to do with the content that harness produces.

This holds symmetrically for the desktop and any future headless consumer of the same feed (a terminal client, an SSH session): the feed profile is headless-consumable by design (adapter-feed-events.md), and a consumer without a webview still owes its own rendering surface the same content-class and confinement discipline this document states, even though CSP enforcement specifically is a webview mechanism this document assumes for the desktop renderer.

## Problem statement

A harness prints a line crafted to look like the product's own approval banner, followed by an OSC hyperlink whose visible text says one thing and whose target says another; a user who trusts the frame clicks. A Markdown note pulled from a shared vault embeds an image tag pointing at a remote server: rendering it would leak the reader's presence and could carry a tracking token, and a raw HTML block would run in the renderer if unstripped. A support export shares scrollback that contains a transformed secret no pattern recognized.

None of these three is a bug in the renderer's happy path. Each surface did what its own content class implies it should: a terminal painted the bytes it received, a Markdown viewer rendered a reference, an export serialized what scrollback held. The failure, in every case, is that rendering was allowed to imply more than display — a byte stream that looks like chrome, a reference that looks like a private fetch, an export that looks complete.

**The renderer shows; it never decides. Everything rendered is data, the renderer holds no capability beyond display and a typed request channel, and no content can navigate, execute, fetch, write the clipboard, or approve without a user gesture that crosses the typed IPC — and, where it matters, a device-local approval.**

Residual risk exists on every surface this document constrains: sanitization and confinement remove a class of attack, but a technically inert rendering can still mislead a rushed reader, and detection of a malformed or hostile sequence is a diagnostic signal, never itself the boundary — the boundary is the allowlist and the typed channel, consistent with [TM-001's binding principle](threat-model.md) that content is never authority. RCS-001's job is to make that boundary hold structurally; it does not claim to make every rendered surface impossible to misread.

## Definitions

| Term | Meaning |
| --- | --- |
| Content class | A named category of rendered content (plain text, rich, terminal, attachment, feed/memory) with its own default rendering mode and allowlist. |
| Rendering mode | The concrete display treatment a content class receives: inert text, sanitized structured markup, or a confined terminal frame. |
| Sanitization allowlist | The enumerated set of elements, attributes, or control sequences a content class may render; anything outside it is stripped or dropped, never passed through by default. |
| Gesture | An explicit user-initiated input event product UI treats as authorization for a bounded action — a click or keypress bound to a declared action — never an event replayed from rendered content. |
| External launch | Opening a target outside the renderer in an OS-level surface (system browser, file manager, other application) following the confirmation this document requires. |
| Quarantine | A landing location outside any workspace where a downloaded or attached artifact is held, unexecuted, until the user chooses an external launch. |
| Redaction manifest | The itemized record a support export carries alongside its content: counts of redacted items by category, and what was excluded. |
| Support export | Any scrollback, diagnostic, or workspace content bundle a user chooses to share for support purposes; passes through the redaction and safe-serialization boundary this document owns. |
| Producer verdict | `trusted` or `untrusted`, per AEC-001's signature verification against a paired key (adopted here unchanged). |
| `ref` / `unresolved` | AEC-001's external-reference shape (`{ kind, id, locator }`) and its `unresolved` rendering state when a reference cannot be dereferenced (adopted here unchanged). |
| Typed IPC | The sole channel by which the renderer requests anything with a side effect from core; every message is typed and validated before it can act (target architecture). |
| Homograph disclosure | Showing an internationalized domain name in its ASCII (punycode) form alongside its display form so a visually similar but distinct target cannot pass unnoticed. |
| Confinement | The property that a content class's rendering cannot escape its own visual boundary — a terminal pane, a reader panel, an entity card — to draw over, replace, or imitate surrounding product chrome. |
| Chrome mimicry | Any rendered content, byte sequence, or control that attempts to reproduce the visual language of the product's own UI — a banner, a dialog, a focus change — so that a user mistakes content for chrome. |
| Reveal | Showing a file's location in the OS file manager without opening, executing, or otherwise acting on it; the only renderer-triggered action available for an executable artifact. |
| Placeholder | The inert stand-in rendered in place of a resource the renderer will not fetch on its own — a remote image, an unhydrated asset — carrying enough information for the user to request the real fetch explicitly. |
| Opt-in scope | The harness or workspace scope unit at which a narrowly-defined relaxation of a default-off policy (an OSC 52 clipboard write, an inline image protocol) is granted; a relaxation never applies globally by default. |
| Startup brief | The bounded facts-and-references package a harness receives after a switch or a claim; always untrusted input under invariant 9, and rendered by this document exactly like any other feed content (adopted from the target architecture and TM-001, unchanged). |
| Support-content path | The single serialization and redaction pipeline every scrollback and support export passes through before it can leave the device, regardless of the originating content class. |
| CSP baseline | The Content-Security-Policy directive set in the CSP baseline section, applied identically to the core renderer and every third-party app. |
| Third-party app | A custom, first-party-built integration running inside the product's extension boundary under ADR-0004, declaring capabilities and content classes and receiving no ambient authority. |
| Allowlist | An enumerated, closed set of permitted values; anything not enumerated is denied by default rather than admitted pending review. Every content class, scheme, and CSP directive in this document is defined as an allowlist, never a denylist. |
| Denylist | An enumerated set of forbidden values, with everything else admitted by default; this document never relies on one, since an unenumerated future addition would otherwise pass through unreviewed. |

## Content classes and rendering modes

Every piece of content the renderer displays belongs to exactly one class. A class determines the default rendering mode; nothing renders richer than its class allows without an explicit, narrower exception stated below. Content arrives already carrying its class — from the typed IPC message that delivered it, the feed event kind that produced it, or the file type a workspace scan recorded — and the renderer never infers a richer class from content shape (a string that looks like Markdown is not thereby rich content; a byte stream that looks like an OSC sequence outside a terminal pane is not thereby terminal control). A content source with no declared class defaults to plain text, the strictest mode this document defines.

This five-class enumeration — plain text, rich, terminal, attachment, feed/memory — is exhaustive for the scope this document drafts. Adding a sixth content class, or widening what a producer or a third-party app may declare, is a change to this document and, where it changes a public surface, a compatibility-relevant change under the [versioning policy](versioning-and-compatibility.md) rather than an implicit runtime extension.

| Class | Default mode | Allowed | Never |
| --- | --- | --- | --- |
| Plain text | Inert text | Text, whitespace normalization | Auto-activated links, embedded markup interpretation |
| Rich (Markdown) | Sanitized structured markup | Headings, paragraphs, lists, emphasis, code spans/blocks, blockquotes, tables, links (inert until gesture), images by logical reference only (Context Catalog locator per AEC-001 design rule 6) | Raw HTML, scripts, iframes, forms, embedded styles; front matter is rendered as data, never interpreted |
| Terminal (PTY output) | Confined terminal frame | Layout controls (cursor, erase, SGR, scroll regions, alternate screen); the OSC policy below | Drawing outside the pane, mimicking product chrome, hosting an approval |
| Artifacts, attachments, downloads | Quarantined landing | Reveal in file manager; external launch with gesture and confirmation | Execution from the renderer; auto-open |
| Feed and memory content | Inert text | `ref` resolution per AEC-001 (`unresolved` shown as such) | Auto-activation; rendering an `untrusted` producer's content as if verified |

Plain text is the default for every source: control characters are stripped, and links render as text with an explicit open affordance rather than auto-activating. Rich content renders only through the Markdown allowlist above; a raw HTML block is always stripped rather than passed through unsanitized. Terminal output is rendered inside a visibly distinct terminal frame; no terminal byte can draw outside its pane or mimic product chrome, and approvals are never rendered inside a terminal pane.

Attachments and downloads land in a quarantined location outside any workspace and are never executed from the renderer; "open on device" is an external launch requiring a gesture and confirmation, and an executable is only ever revealed in the file manager, never launched. The quarantine location is a dedicated directory outside any workspace by default; carrying a heavy artifact through the vault's heavy-asset tier by reference instead is an open decision (D3), not a default this document assumes.

The quarantine directory itself lives under the product's own application-data directory, never inside a workspace: if the OS's default download location would otherwise fall inside any registered workspace root — including a home directory a user has registered as a workspace — the product redirects the download to its own quarantine directory rather than letting an OS default silently land inside content the product treats as workspace state.

Every quarantined file name is sanitized before it is shown or referenced: path-traversal sequences, reserved device names, right-to-left override characters, and excessive length are all normalized or rejected. An executable is identified by the OS's own extension list, magic-byte inspection, and — where the platform provides one — its quarantine or mark-of-the-web flag; on a platform with no such equivalent, the product removes execute permission on the file itself and records the download's provenance in its own metadata rather than claiming a platform guarantee that does not exist there. An archive is never auto-extracted: extraction is an explicit user action, protected against path traversal and symlink escape, and an executable found inside an extracted archive is reveal-only exactly like any other executable. A maximum download size applies (open decision D12).

Reveal-only for executables is deliberate, not incidental: a downloaded or attached binary is exactly the kind of content the renderer cannot itself judge safe to run, and handing execution to the OS file manager — the surface the user already trusts to launch arbitrary local files — keeps that judgment where the user actually exercises it, rather than adding a second, renderer-mediated "run" affordance this document would then have to defend.

Feed and memory content — events, observations, alerts, startup briefs — renders as plain text by default; `ref` objects resolve per AEC-001, with `unresolved` shown as such rather than as a broken link or a guess. A startup brief is always untrusted input under the target architecture's invariant 9, and RCS-001 renders it exactly as it renders any other feed content — never with elevated trust because of its role in a harness switch. A producer whose verdict is `untrusted` renders with that verdict visible and every action on it disabled; this is the same rule TM-001's CNT-3 credits as mitigation, restated here as a rendering rule rather than a claim protocol rule. A feed alert (`alert.raised`) renders through this same plain-text default; its mirrored presence in the feed is never the only delivery path, since alerting depends on the out-of-band dispatch adapter-feed-events.md defines, not on a rendered surface being open ([TM-001 CNT-4](threat-model.md)).

## Terminal control and OSC policy

AEC-001 normalizes only allowlisted terminal controls into typed actions and renders unsupported sequences inert or drops them with diagnostics (target architecture, invariant 6; AEC-001). RCS-001 states the policy that normalization enforces.

| Sequence family | Policy | Rendering |
| --- | --- | --- |
| Title (OSC 0/2) | Sanitized text, capped at 256 characters, control and bidirectional-override characters stripped | Pane header only |
| Hyperlinks (OSC 8) | Inert by default | Target shown on hover/focus; opened only through the navigation rules below |
| Clipboard (OSC 52) | Write disabled by default; per-scope opt-in plus a transient per-write acceptance and rate limit (D2); query intercepted and dropped; read never | No visible affordance unless the scope has opted in; a query returns no clipboard value to the harness |
| Notifications (OSC 9 and common desktop-notification variants) | Sanitized text, capped at 256 characters, control and bidirectional-override characters stripped | Surfaced as a feed alert, never as a system prompt |
| File transfer and image protocols (the iTerm2-style OSC 1337 family, Sixel, Kitty graphics) | Disabled by default | Images may be enabled per scope as inert bitmaps only (open decision D1) |
| DCS/APC/PM/SOS strings | Ignored, bounded (D11) | No rendering effect; aborted on overflow or terminator timeout |
| Unknown or malformed sequences | Dropped | Counted in diagnostics |

Terminal content is confined to its frame with the producer identity shown alongside it; no terminal byte can produce product chrome, a dialog, or a focus change. This is the defense against a harness printing a line crafted to look like the product's own approval banner — the frame itself, not sequence-by-sequence inspection, is what makes the forgery visible as terminal content rather than product chrome. This confinement is binding, not advisory (RCS-001-R5): the pane frame's visual appearance belongs to [context-orb.md](context-orb.md)'s presentation design, but the invariant that no content can disguise or escape that frame belongs to this document and holds identically across every theme the product ships. Each default in the policy table above follows the same reasoning TM-001 applies to PRC-2 and HAR-5: only allowlisted controls normalize into typed actions, an unsupported sequence is inert or dropped rather than passed through optimistically, and a family with an obvious authority-equivalent reading — clipboard write, file transfer, a fabricated system notification — starts disabled and requires an explicit opt-in rather than an explicit opt-out. Notifications route to the feed's out-of-band alert path rather than an OS-level prompt for the same reason TM-001's CNT-4 credits: alert delivery must not depend solely on a terminal pane being visible, and a sanitized feed alert cannot itself be mistaken for an OS security prompt. Title and notification text is capped and stripped as the table states; a domain-like substring within either renders in its ASCII (punycode) form, the same disclosure Navigation and URLs requires for a link target, rather than a Unicode rendering that could disguise it.

OSC 52's query form — asking the terminal to report clipboard content back to the harness — is intercepted and dropped before it reaches any terminal-emulation layer; clipboard content never enters the PTY input stream, regardless of scope opt-in, because a query is a read and this document's clipboard read-never rule (Clipboard) admits no per-scope exception. A write, once a scope has opted in, still requires a transient per-write acceptance — a short-lived, one-tap confirmation shown at the moment of the write, not a standing capability the opt-in alone grants — and is rate-limited; opting a scope in relaxes the default-off posture, it does not authorize an unbounded stream of silent writes (open decision D2).

Every DCS, APC, PM, SOS, and OSC string this policy recognizes is bounded by a maximum length and a terminator timeout (open decision D11, default 4 KiB and the terminal's own reasonable inter-byte timeout); on overflow or timeout the sequence is aborted and every subsequent byte renders as plain text rather than continuing to accumulate as an unterminated control sequence. No sequence this document allows can swallow more than that bound.

Entering the alternate screen is allowed and carries a visible indicator in the pane header so a user can tell which buffer is showing; the main screen's scrollback is preserved underneath and restored exactly when the alternate screen exits. An export (Redaction and safe serialization for scrollback and support exports) includes both buffers rather than only whichever was visible at export time.

A vendor-specific escape sequence outside the enumerated families — a proprietary iTerm2 extension, a tmux passthrough wrapper, a Konsole- or Kitty-specific control not already covered above — falls under "unknown or malformed" and is dropped with diagnostics by default, the same as a genuinely malformed sequence. Extending the allowlist to a new vendor sequence is a change to this document, evaluated against the same authority-equivalent-reading question every family above was, never an implicit runtime allowance.

## Navigation and URLs

Navigation is two separate paths with two separate scheme allowlists: in-renderer navigation, which never leaves the renderer's own document, and external launch, which hands a target to an OS-level surface. A deep link into a registered application is external launch, not in-renderer navigation, and each path states its own allowlist below.

**In-renderer navigation.** The renderer resolves a navigation target against a scheme allowlist: `https`, `mailto`, and product-internal logical references (AEC-001 `ref`). `http` requires confirmation and is shown as insecure. `file` is never navigated in-app; it resolves only to "reveal" via external launch. `javascript`, `data`, `blob`, and any scheme outside this allowlist never resolve in-renderer. The renderer never navigates its own document to a different top-level resource; the CSP and typed IPC policy below enforce that structurally rather than by convention alone.

**External launch.** A target opens outside the renderer, in an OS-level surface, only through this separate path, with its own allowlist: `https`, `mailto`, and an application scheme registered on this device by an approved app or adapter — a device-local registration list, never an ambient OS-handler probe; an unregistered scheme never launches (open decision D10 states the registration source). Every external launch — an inline link, a deep link, or the dashboard's own launch surface below — requires the same explicit confirmation: the full target shown, an internationalized domain name rendered in its ASCII (punycode) form alongside the display form, and any mismatch between visible link text and the actual target called out. A provider may be allowlisted per scope to skip the prompt after a first confirmation (open decision D5); the default before that decision ships is to confirm every time. Once a confirmed launch hands a target to the system browser, any redirect chain the target's own server performs is the system browser's responsibility, not this document's — RCS-001's guarantee ends at the confirmed handoff.

`javascript`, `data`, `blob`, `vbscript`, and every scheme outside both allowlists above are removed from `href` and `src` attributes at sanitization time in core, before content reaches the renderer at all — the renderer receives only already-sanitized content and never holds a disallowed scheme to begin with. The click-time scheme checks stated above are defense in depth on top of that parse-time removal, never the only gate against them.

This confirmation rule is not limited to inline links. The dashboard's "External launch" surface pattern — opening a generated artifact, a micro-app quick link, or an application deep link from a gadget or the Orb's entity card — is navigation, not execution, and follows exactly the external-launch allowlist and confirmation stated above; the dashboard never auto-launches one as a side effect of rendering ([context-orb.md](context-orb.md); [TM-001 CNT-5](threat-model.md)).

`mailto` requires no confirmation beyond the OS's own compose-window handoff on either path, since it never fetches or renders anything back into the renderer.

Each allowlist is maintained in this document and re-evaluated whenever the underlying desktop framework changes what schemes a webview recognizes or the OS changes how application schemes register, mirroring the discipline TM-001's GIT-5 already applies to Git remote transports. A new OS-level or application scheme is untrusted by default until this document is updated to admit it, or, for an application scheme, until it appears on the device-local registration list D10 defines; recognizing a scheme is never itself a reason to trust it.

## Images and remote resources

The renderer never fetches a remote resource of any kind. The CSP baseline's `img-src` is limited to the app origin and a local artifact scheme; `connect-src` is `'none'`, and `font-src` admits only bundled, self-hosted fonts — no external font service load, which would itself be a remote fetch with the same presence-leak shape as a tracking image. A remote image referenced from rich content renders as a placeholder showing the URL as text, with an explicit "fetch through the core into the artifact tier" action rather than an automatic load (open decision D1 states the default for that action). This is the mitigation for a Markdown note whose embedded image tag points at a remote server: rendering it directly would leak the reader's presence and could carry a tracking token in the URL, so nothing fetches until the user asks. Product-generated SVG (Orb projections) is either sanitized — no scripts, no external references — or rasterized before it reaches the renderer (open decision D7).

The same no-renderer-fetch rule applies to any embedded media reference, not only images: the Markdown allowlist in Content classes and rendering modes never includes a `<video>`, `<audio>`, `<iframe>`, or `<object>` element, and the CSP baseline's `object-src 'none'` and `frame-src 'none'` back that allowlist decision structurally rather than leaving it to sanitizer diligence alone.

A locally hydrated artifact — one the Context Catalog already reports as local, per the target architecture's tiered heavy-asset model — is not a "remote resource" for this section's purpose: it loads through the `artifact:` scheme the CSP baseline's `img-src` already admits, with no network fetch involved. The distinction this document draws is remote versus local, not image versus any other media type.

## Clipboard

A clipboard write occurs only on an explicit user gesture from product UI — a copy action or an explicit selection — or through an OSC 52 write within a scope the user has opted into, confirmed at the moment of the write by a transient, rate-limited per-write acceptance rather than a standing capability the opt-in alone grants (Terminal control and OSC policy; open decision D2). The renderer never reads the clipboard, from any content source, and an OSC 52 clipboard query is intercepted and dropped before it reaches any terminal-emulation layer — clipboard content never enters the PTY input stream. Read access is withheld unconditionally rather than gated, because a clipboard read is an exfiltration primitive with no legitimate rendering purpose: nothing this document defines needs the renderer to know what the OS clipboard currently holds.

A paste into a harness is wrapped in bracketed-paste markers when the application has requested bracketed-paste mode, so the receiving program can distinguish pasted text from typed input. When the application has not requested bracketed paste, a paste containing one or more newlines requires an explicit confirmation that shows the content before it is sent — an unconfirmed multi-line paste could otherwise smuggle more than the user glanced at past the first line.

Text pasted into a harness is labelled as user-provided untrusted input, consistent with TM-001's provenance labelling, regardless of whether it was bracketed or confirmed: pasted text is content like any other once it leaves the OS clipboard and enters a harness's context, and it carries no elevated trust for having passed through the user's hands rather than a rendered surface.

## CSP baseline

Every mechanism above depends on the renderer actually being unable to do what its content class forbids, not merely being instructed not to. The CSP baseline is that structural backstop: even a sanitization defect in the Markdown or terminal pipeline cannot turn into script execution, an unapproved network fetch, or a framed browsing context, because the policy blocks the underlying browser capability regardless of what markup or script reached the DOM. The following directives apply to every renderer surface, including a third-party app's:

| Directive | Value |
| --- | --- |
| `default-src` | `'none'` |
| `script-src` | `'self'`, bundled and hashed scripts only, no inline |
| `style-src` | `'self'` plus a nonce for theme variables |
| `img-src` | `'self' artifact:` (local artifact scheme) |
| `media-src` | `'self' artifact:` (local artifact scheme) |
| `font-src` | `'self'` |
| `manifest-src` | `'self'` |
| `connect-src` | `'none'` (typed IPC only) |
| `frame-src` | `'none'` |
| `object-src` | `'none'` |
| `base-uri` | `'none'` |
| `form-action` | `'none'` |
| `worker-src` | `'self'` |

`script-src 'self'` with bundled, hashed scripts excludes both an external CDN load and runtime `eval`-style execution; a script the renderer runs must have shipped inside the application bundle and matched its hash, never have been fetched, generated, or evaluated at runtime from any content source. No embedded browsing context is permitted. The `style-src` nonce covers only product-defined theme-variable values — the curated accent palette [context-orb.md](context-orb.md) defines — never a style computed from or influenced by rendered content; no content source can inject a value that reaches the nonce'd style block.

`connect-src 'none'` governs network-shaped requests a browsing context could otherwise issue — `fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource` — and does not govern the typed IPC channel itself: typed IPC rides the desktop framework's own bridge API (ADR-0002), not any of those primitives, and is not a network request this directive was designed to gate. Where a platform's bridge implementation requires a custom protocol handler to carry that traffic, the protocol is added to `connect-src` explicitly, documented in this section, and verified under VP-001 — never left as an implicit, undocumented exception to the baseline above.

The baseline is enforced as the renderer's actual CSP policy — a header or an equivalent webview policy under the desktop stack ADR-0002 selects — not as guidance a component may opt out of, and RCS-001-R13 makes it a conformance-tested acceptance gate rather than an aspiration. A third-party app inherits this baseline exactly, with no relaxation (ADR-0004): a custom app's content class rules, CSP, and navigation constraints are the same ones this document states for the core renderer, and no app manifest field can widen a directive.

## Typed IPC constraints

Every renderer→core request is a typed message. A payload carries a logical reference — never a raw filesystem path, consistent with [HTP-001-R18](handoff-transaction-protocol.md) and AEC-001 design rule 6. Core validates every request and routes anything with a side effect through the approval path where one applies; the renderer only ever receives projections and content already classified by content class and producer verdict. This is the structural half of "the renderer shows; it never decides": the renderer has no capability to act on content directly, only to ask core to act on its behalf.

This is also the concrete form the target architecture's invariant 4 takes at the renderer boundary: no generic shell and no unrestricted filesystem capability means, in practice, that every renderer-initiated effect is one of the finite typed IPC message kinds core recognizes, routed through the same bounded ports diagram — `ProcessSupervisor`, `GitCoordinator`, `MemoryCoordinator`, `KnowledgePort`, `BlobStorePort`, `DeliveryPort`, `SecretStore` — that the target architecture defines for the framework-independent domain. Nothing in this document grants the renderer a new port; it only constrains what the renderer may put on the wire toward the ports that already exist.

A malformed or schema-invalid renderer request is rejected before it reaches any port. This document does not define the validation mechanism itself — that binding is ADR-0002's and AEC-001's — but requires that rejection, not best-effort interpretation, is the default outcome for a request outside the typed schema. A rejected request surfaces to the renderer as `failed`, consistent with the Signal mapping below; it never silently drops and never retries with a relaxed interpretation of the same payload.

## Redaction and safe serialization for scrollback and support exports

One serializer produces every support export and scrollback export, including a terminal pane's own scrollback: the same redaction and safe-serialization boundary applies whether the source content class was rich, plain, or terminal (target architecture, Content, secrets, update, and Git trust). Redaction covers known structured fields — secret references by custody inventory, tracked secret values, identity evidence — plus pattern-based candidates shown as a warning, since detection cannot prove absence ([TM-001 SEC-3](threat-model.md)).

Raw vendor output is excluded from an export by default and included only with an explicit per-export opt-in shown with a warning. Every export carries a redaction manifest — counts by category, what was excluded — and a plain statement that arbitrary output may still leak transformed secrets. This is the direct answer to a support export sharing scrollback that contains a transformed secret no pattern recognized: the manifest and the disclosure make that residual risk visible rather than implied, instead of a silent guarantee this document cannot actually make. The user previews the export before sharing it, and that preview renders the post-redaction content exactly as it will be shared — never a raw view redaction is applied to only afterward. The finished export file lands in the same quarantine location Content classes and rendering modes defines for any other downloaded or generated artifact, not directly in a workspace. Where the manifest itself needs to reference a path, it carries that reference in redacted or logical form, consistent with the typed IPC rule against raw filesystem paths (Typed IPC constraints) — a manifest that named real on-disk paths verbatim would leak exactly the kind of detail this document redacts everything else to protect. The Engram Cloud token and device keys never appear in any export ([RSP-001 credential custody](roaming-and-engram-sync.md); [HTP-001-R18](handoff-transaction-protocol.md)).

A further redaction category covers sensitive metadata rather than secret values: scope and workspace names, device identifiers, remote URLs, and user names are redacted by default, since a support export can identify a person or an environment even when it contains no credential at all. This category has its own explicit per-export opt-out — a user who needs to share it for diagnosis can, but only by choosing to — and the manifest discloses whenever that opt-out was exercised, the same way it discloses a raw-vendor-output inclusion.

Safe serialization is a rendering rule as much as a redaction rule: an exported terminal scrollback is serialized as text with its control bytes escaped or stripped, never replayed as live terminal control on the receiving end — an export that carried live OSC/escape sequences would hand a second, unconfined rendering surface to whatever produced the original bytes, defeating the confinement this document establishes for the live pane in the first place. The same serializer applies whether the export's source content class was rich, plain, or terminal; there is no export-time exception for any class.

Redaction's structured-field category tracks the same custody classes the target architecture and TM-001 already define — `OS-secret-store`, `Engram-managed-local`, `unprovisioned` — so a value known to be secret by custody inventory is removed with certainty, while everything else falls to the pattern-based, best-effort category disclosed above. RCS-001 does not define a new secret taxonomy; it consumes the one custody already establishes.

## Third-party apps and accessibility interplay

A third-party app declares which content classes it renders and receives no relaxed CSP or navigation rule beyond what this document states (ADR-0004). The declaration is part of the same capability manifest ADR-0004 and TM-001's actor A3 already assume exists; RCS-001 adds no separate content-security manifest, only the content-class field within that one manifest. Declaring a class it does not actually need is a manifest-accuracy question TM-001's PRC-3 already tracks; RCS-001 only guarantees that whatever a declared class is permitted to do, an app gets no more.

An app's own code is bundled and hashed by the product at install time (ADR-0004; signing per UTA-001 once accepted), the same treatment `script-src 'self'` requires of the core renderer's scripts — an app never ships script the CSP baseline would not already admit from the core. An app MUST NOT open a browser window, a webview, or any browsing context outside the renderer policy this document states; every surface an app presents, including one it spawns itself, inherits the CSP baseline and navigation rules with no exception (extends RCS-001-R17).

Sanitization never removes textual content: plain-text fallback and reading order are always preserved, regardless of content class or the app rendering it. VOC-001 owns voice and text-fallback behavior generally; alt text is text, and this document sanitizes it like any other text rather than stripping it as markup. A sanitizer that strips a disallowed element MUST preserve that element's own text content in reading-order position rather than discarding the whole node — the allowlist governs structure and interactivity, never the words themselves.

Where VOC-001 defines voice output reading from rendered content, it reads the same sanitized, reading-order-preserved text this document guarantees exists; VOC-001 does not receive raw markup, an unstripped attribute, or a terminal escape sequence as an input to synthesize.

## Product requirements

Each requirement is an acceptance gate with a testable condition.

| ID | Requirement |
| --- | --- |
| RCS-001-R1 | Every content class not explicitly upgraded to a richer mode MUST render as plain text by default: control characters stripped, links inert until an explicit open gesture. |
| RCS-001-R2 | Rich (Markdown) content MUST render only through the allowlist in Content classes and rendering modes; raw HTML, embedded styles, scripts, iframes, and forms MUST be stripped and MUST NOT reach the DOM. |
| RCS-001-R3 | The renderer MUST NOT execute a script, load an iframe, or render a form sourced from any content class, regardless of producer verdict. |
| RCS-001-R4 | PTY output MUST render only through the terminal control allowlist and the OSC policy table; OSC 52 clipboard write, and the OSC 1337/Sixel/Kitty file-transfer and image protocols, MUST default off, and OSC 9 and the desktop-notification variants MUST surface as a sanitized feed alert rather than a system prompt. |
| RCS-001-R5 | The terminal pane frame MUST be undisguisable: it MUST carry visible producer identity, its chrome MUST be product-controlled and MUST NOT be themeable or overridable by rendered content, and no terminal byte sequence MAY draw outside the pane, mimic product chrome, or move UI focus. The frame's visual appearance is [context-orb.md](context-orb.md)'s to design; this invariant — that the frame cannot be disguised or escaped — is RCS-001's and does not vary by theme. |
| RCS-001-R6 | An approval MUST NOT be rendered inside a terminal pane. |
| RCS-001-R7 | In-renderer navigation MUST be restricted to the scheme allowlist (`https`, `mailto`, product-internal `ref`; `http` only with confirmation and shown as insecure); the renderer MUST NOT navigate its own document to a different top-level resource. |
| RCS-001-R8 | An external launch MUST be restricted to `https`, `mailto`, and an application scheme registered on this device by an approved app or adapter (open decision D10); it MUST require an explicit user confirmation that shows the full target, renders an internationalized domain name in its ASCII form, and calls out any mismatch between visible link text and the actual target. |
| RCS-001-R9 | The renderer MUST NOT fetch a remote resource of any kind; a remote image referenced from rich content MUST render as a placeholder pending an explicit user-initiated fetch through core. |
| RCS-001-R10 | An attachment or download MUST land in a quarantined location outside any workspace and MUST NOT be executed from the renderer under any circumstance. |
| RCS-001-R11 | An executable file MUST be revealed only in the OS file manager from any renderer surface; the renderer MUST NOT launch it directly. |
| RCS-001-R12 | A clipboard write MUST occur only on an explicit user gesture from product UI, or through an OSC 52 write within a scope the user has opted into and confirmed with a transient, rate-limited per-write acceptance; opt-in alone MUST NOT grant a standing write capability. The renderer MUST NOT read the clipboard from any content source, and an OSC 52 query MUST be intercepted and dropped before it reaches clipboard content. |
| RCS-001-R13 | The CSP baseline in the CSP baseline section MUST be enforced on every renderer surface, including a third-party app's, and MUST be covered by the acceptance evidence conformance tests. |
| RCS-001-R14 | Every renderer→core request MUST be a typed IPC message; a payload MUST carry a logical reference and MUST NOT carry a raw filesystem path. |
| RCS-001-R15 | A support export or scrollback export MUST pass through one serializer that produces a redaction manifest and a user preview before the export can be shared; raw vendor output MUST be excluded from the export by default. |
| RCS-001-R16 | Content whose producer verdict is `untrusted` (AEC-001) MUST render with that verdict visible and MUST have every action on it disabled. |
| RCS-001-R17 | A third-party app MUST inherit this document's CSP and navigation rules with no relaxation, on every surface it presents including one it spawns itself; an app's own code MUST be bundled and hashed by the product at install time, and an app MUST NOT open a browser window, webview, or browsing context outside the renderer policy this document states. |
| RCS-001-R18 | Sanitization MUST NOT remove textual content; plain-text fallback and reading order MUST be preserved for every content class this document defines. |

## Signal mapping

| Condition | RCS-001 state | Public vocabulary | Consequence |
| --- | --- | --- | --- |
| Unsupported or malformed terminal control sequence | Dropped | `failed` (per-sequence, diagnostics only) | Sequence discarded; counted in diagnostics, never rendered |
| Blocked navigation (scheme, in-app target) | Inert | `failed` | Link never activates; reason shown on hover/focus |
| Blocked download or execution attempt | Quarantined, unexecuted | `failed` | Artifact retained in quarantine; execution refused |
| Remote resource referenced from rich content | Placeholder | `unresolved` | URL shown as text; explicit fetch-through-core action offered |
| Redaction applied to an export | Manifest attached | — (export carries its own manifest) | User previews counts and exclusions before sharing |
| Producer verdict `untrusted` | Rendered with verdict shown | `untrusted` | Content visible; every action on it disabled |
| Raw HTML or script encountered in rich content | Stripped | `failed` (per-node, diagnostics only) | Node removed; its text content preserved in place per Third-party apps and accessibility interplay |
| Clipboard write attempted outside a gesture or opted-in scope | Refused | `failed` | No system clipboard mutation occurs |
| Support export requested with raw output included | Warned, not blocked | — (export not yet shared) | Warning shown in the preview; user confirms before the export leaves the device |
| Renderer unavailable | Full text and recovery control preserved | `uncertain` (assigned by RCS-001; the failure table itself names no token) | Target architecture's required failure table states this condition verbatim as "preserve full text and recovery control" |

## Open decisions

| ID | Question | Options | Default proposal |
| --- | --- | --- | --- |
| D1 | What is the default action on a remote image reference in rich content? | Placeholder only, no fetch action; placeholder plus explicit fetch through core into the artifact tier; auto-fetch with a warning | Placeholder plus explicit fetch through core into the artifact tier |
| D2 | What scope does an OSC 52 clipboard-write opt-in cover, and what confirms each individual write? | Per session; per scope; global toggle — combined with either a standing capability once opted in, or a transient per-write acceptance | Per scope, default off, plus a transient per-write acceptance (short window, one-tap) and a rate limit on every write; opt-in is never a standing capability |
| D3 | Where do downloads land? | A quarantine directory outside any workspace; the vault's heavy-asset tier by reference | Quarantine directory outside any workspace |
| D4 | What Markdown subset does the rich allowlist support? | CommonMark subset plus tables; full CommonMark; a narrower list without tables | CommonMark subset plus tables |
| D5 | When is external-link confirmation required? | Always; per-scope allowlisted providers skip after a first confirmation | Always, in v1; per-scope allowlisting is a later relaxation |
| D6 | Is raw vendor output ever included in a support export? | Never; per-export opt-in with a warning; included by default | Per-export opt-in with a warning; excluded by default |
| D7 | How does product-generated SVG (Orb projections) reach the renderer? | Sanitized (no scripts, no external references); rasterized before delivery | Sanitized |
| D8 | How do terminal notification sequences surface? | As a feed alert; as an OS-level notification | Feed alert |
| D9 | Does content-level renderer state need dedicated public vocabulary tokens (for example `inert`, `quarantined`), or does it reuse `failed`/`uncertain`/`unresolved`? | Add dedicated tokens through the compatibility policy; reuse the existing vocabulary | Reuse `failed`, `uncertain`, and `unresolved` until a dedicated token is proposed through the compatibility policy, consistent with [TM-001's D8](threat-model.md) |
| D10 | What is the registration source for an application scheme eligible for external launch? | Approved-app manifest (ADR-0004 capability declaration); adapter declaration (AEC-001 producer identity); OS handler list (platform-reported, unauthenticated) | Approved-app manifest — a scheme is added to the device-local registration list only when an approved app's own manifest declares it at install/approval time, never inferred from an OS-reported handler list alone |
| D11 | What is the maximum length and terminator timeout for a DCS/APC/PM/SOS or OSC string before it is aborted? | Fixed byte bound only; fixed bound plus a terminator timeout; no bound (rely on ECMA-48 conformance) | 4 KiB maximum length plus a terminator timeout; on overflow or timeout the sequence aborts and subsequent bytes render as plain text |
| D12 | What is the maximum size for a single download or attachment? | No limit; a fixed size cap; a user-configurable cap with a safe default | A fixed default cap, exact figure pending VP-001 evidence on quarantine-directory disk behavior; configurable per the support matrix once VP-001 lands |

## Acceptance evidence and follow-up

The checklist that satisfies the renderer content-security gate the roadmap names for Alpha (renderer content-security baseline) and Alpha → Beta (renderer-content review):

- CSP conformance: no network request the CSP does not allow, no inline script, no embedded browsing context, on every renderer surface including a third-party app's.
- A Markdown corpus test: HTML, script, and style are always stripped; images render only by local or Context Catalog reference, never a direct remote fetch.
- A terminal corpus test: OSC 8, OSC 52, the OSC 1337 family, title, and notification sequences behave per the policy table; unknown sequences are dropped and counted; a dedicated chrome-mimicry test confirms no terminal byte sequence can draw outside its pane or forge product chrome.
- A control-sequence bound test: a DCS, APC, PM, SOS, or OSC string exceeding the D11 length bound, or left unterminated past the terminator timeout, aborts rather than continuing to accumulate, and every subsequent byte renders as plain text.
- An alternate-screen test: entering the alternate screen shows a visible pane-header indicator, the main screen's scrollback is preserved underneath and restored exactly on exit, and an export produced while either screen was active includes both buffers.
- A terminal frame undisguisability test (RCS-001-R5): the pane frame renders with visible producer identity and product-controlled, non-content-themeable chrome under every shipped theme, captured by screenshot, and no terminal byte sequence alters the frame itself.
- Navigation tests: in-renderer navigation is restricted to its allowlist and never navigates the renderer's own document to a different resource; `javascript`, `data`, `blob`, and every non-allowlisted scheme are confirmed removed from `href`/`src` at core sanitization time before the renderer receives the content.
- Download tests: an attachment lands in the product's own quarantine directory — even when the OS default download location would otherwise fall inside a registered workspace root — is never executed from the renderer, and an executable is reveal-only; a filename with a path-traversal sequence, a reserved device name, an RTL-override character, or excessive length is sanitized before use; an executable is identified by extension list, magic bytes, and the OS quarantine flag where available, with a platform lacking that flag falling back to execute-permission removal and product-recorded provenance; an archive never auto-extracts, and an executable revealed from inside one is reveal-only; a download over the D12 size cap is refused.
- Clipboard tests: a write requires a gesture, or an opted-in OSC 52 scope plus a transient per-write acceptance within its rate limit; an OSC 52 query is intercepted and dropped before reaching any terminal-emulation layer and clipboard content never enters the PTY input stream; reads never succeed from any content source; a paste is wrapped in bracketed-paste markers when requested, and an unbracketed multi-line paste requires confirmation showing its content before it is sent.
- Redaction export tests: known structured fields and the sensitive-metadata category are removed, the manifest's counts match the redaction actually applied, raw vendor output is excluded by default, the export preview renders post-redaction content only, the finished export file lands in the product's quarantine location rather than a workspace, any path the manifest itself references appears in redacted or logical form, and the Cloud token and device keys never appear in any export.
- A third-party app inheritance test: an app's CSP and navigation behavior match the core renderer's with no relaxation.
- An accessibility preservation test: sanitization never drops textual content or reading order for any content class.
- A producer-verdict rendering test: `untrusted` content renders with the verdict visible and every action on it disabled, across every content class, not only feed events.
- An external-launch surface test: the dashboard's artifact, micro-app, and deep-link launches are restricted to `https`, `mailto`, and application schemes on the device-local registration list (D10), require the same confirmation as an inline link, and none auto-launches as a side effect of rendering; an unregistered scheme never launches from any surface.
- A remote-resource placeholder test: a remote image, external font, or other cross-origin fetch attempted from rendered content never leaves the device; the placeholder and explicit fetch-through-core action are the only path to hydration.
- A homograph disclosure test: an internationalized domain name renders its ASCII form alongside the display form, and a visible-text/target mismatch is flagged, at every external-launch confirmation.
- A vocabulary conformance test: every Signal mapping condition renders using an existing public state token unless open decision D9 has been decided and the compatibility policy updated to carry the new token.
- A scrollback-export replay test: an export containing terminal control bytes never re-executes them on the viewing surface that opens it; every control byte is escaped or stripped by the same serializer live rendering uses for confinement.
- A governance check: acceptance findings are recorded as Security Reviewer advice to the Project Owner at the roadmap's Alpha → Beta gate, per the promotion and approval gates table — not a self-certified pass.
- A content-class exhaustiveness check: no rendered surface presents a content class outside the five this document enumerates, and any addition is traced to a compatibility-policy change rather than an undocumented runtime allowance.

Renderer content-security review is a precondition for TM-001's own acceptance, not a substitute for it (mirroring [TM-001's acceptance evidence](threat-model.md)): TM-001 credits this document's mitigation for B1, CNT-1..6, HAR-5, and PRC-2, and its own acceptance checklist cannot close while this checklist is open.

Debt, not drafted here. This document does not specify the exact sanitizer library or CSP-header delivery mechanism the desktop stack uses to enforce the baseline — that binding is VP-001's evidence to produce against ADR-0002's pinned platforms. Open decisions D1 through D12 remain the owner's calls; a feature that assumes their default proposal before the owner disposes must say so rather than presenting the default as decided.

## Related contracts

- [Target architecture](target-architecture.md) — invariant 4 (no generic shell or filesystem capability), invariant 9 (synchronized content untrusted), the trust-boundary diagram (untrusted renderer → typed IPC), executable approval, PTY, content/secrets/redaction, and required failure states.
- [Threat model](threat-model.md) (TM-001) — the attacker model this contract enforces the boundary for; B1, CNT-1..6, HAR-5, PRC-2, and TM-001-R7 on approval-surface display.
- [Adapter feed event schema](adapter-feed-events.md) (AEC-001 feed profile) — the `ref`/`unresolved` reference shape, producer identity and the `untrusted` verdict, and PTY normalization into typed actions this document's terminal policy constrains.
- [Handoff transaction protocol](handoff-transaction-protocol.md) (HTP-001) — HTP-001-R18, the shared rule that no export or portable payload carries secrets, tokens, device keys, or raw filesystem paths.
- [Workspace roaming and Engram sync protocol](roaming-and-engram-sync.md) (RSP-001) — credential custody; the Cloud token never appearing in a support bundle.
- [ADR-0002: Desktop technology stack](adr/0002-desktop-technology-stack.md) — the renderer/IPC/PTY/content rules this document drafts in full, and the acceptance evidence naming this document as the "renderer-content review."
- [ADR-0004: Fully open platform with custom integrated apps](adr/0004-open-platform-and-custom-apps.md) — the third-party app boundary this document's CSP and navigation rules extend without relaxation.
- [ADR-0003: Local Markdown and tiered heavy assets](adr/0003-local-markdown-and-tiered-assets.md) — the Context Catalog and heavy-asset tier this document's image-placeholder and quarantine-by-reference open decisions (D1, D3) resolve against.
- [Context Orb specification](context-orb.md) — the reader panel, terminal frame, entity card, and external-launch presentation this document's content classes and navigation rules constrain.
- [Versioning and compatibility](versioning-and-compatibility.md) — the public state vocabulary this document's Signal mapping reuses, and the compatibility surface open decision D9 would extend.
- [Product roadmap](roadmap.md) — the Alpha renderer content-security baseline and the Alpha → Beta renderer-content review gate this document's acceptance satisfies.
- Voice interaction contract (VOC-001) — accessibility text fallback and reading order this document's sanitization rule (RCS-001-R18) defers to; not yet drafted.
- Update trust architecture (UTA-001) — key rotation and compromise recovery for the product's own updates, distinct from and unrelated to rendered content; not yet drafted.
- Governance and support matrix (GOV-001) — named roles and approval authority this document assumes without redefining; not yet drafted.

## References

- [Content Security Policy Level 3](https://www.w3.org/TR/CSP3/) — the directive set the CSP baseline table draws on.
- [CommonMark specification](https://spec.commonmark.org/) — the Markdown subset the rich content-class allowlist is scoped against.
- [XTerm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) — the OSC and control-sequence families the terminal control policy classifies.
- [ECMA-48: Control Functions for Coded Character Sets](https://www.ecma-international.org/publications-and-standards/standards/ecma-48/) — the underlying standard the OSC/CSI/DCS/APC/PM/SOS sequence families in the terminal control policy are built on.
- [Unicode Technical Report #36: Unicode Security Considerations](https://unicode.org/reports/tr36/) — background on homograph and internationalized-domain-name spoofing behind the external-launch confirmation's disclosure rule.
- [OWASP Top 10 for Large Language Model Applications](https://owasp.org/www-project-top-10-for-large-language-model-applications/) — the same background categorization TM-001 draws on, relevant here to insecure-output-handling patterns a renderer sanitizer must resist.
- [WHATWG URL Living Standard](https://url.spec.whatwg.org/) — scheme and origin parsing behavior the navigation scheme allowlist assumes.
- [Target architecture](target-architecture.md) — the trust-boundary diagram and invariants this document's mechanics implement.
- [Threat model](threat-model.md) — the attacker model and threat catalog this document's mitigations are credited against.
