# ADR-014 Teams: next after Word/Excel; custom protocol first, Matrix as a later bridge

Status: proposed, 2026-10-11. Owner decision (2026-10-11): Teams, client and server, is the next product after Word and Excel, and must be cross-platform including mobile. Related: ADR-006, ADR-008, ADR-009, ADR-010, `adr-sharing.md`, `adr-ui-platform.md`, `system-context.md`, `../security/threat-model.md`, `../roadmap/first-milestones.md`. The decision-log entry is ADR-014 (the brief called it ADR-011, which is the locale ADR).

Source note: licence and platform facts come from the primary sources listed at the end, as known on 2026-10-11, and were not re-fetched for this draft. Re-check items marked "verify" before freezing.

## 1. Context and scope

Teams parity covers chat, channels, threads, presence, file tabs, meetings with audio, video and screen share, and calendar. The server must be operator-hosted (ADR-009) and share identity, tenancy and storage with the sharing server (`adr-sharing.md`, Stage 1). Clients follow ADR-010: one Rust core, a native UI per platform.

## 2. Server architecture

**Decision: extend `open365-server`. It stays a single Rust binary** (axum on tokio, rustls), with Teams as modules behind the existing permission function. No second service is needed for chat.

- **Tenancy:** `tenant_id` on every row and key path, taken from the session (`TenantCtx`), never from the request. Guests are principals with a restricted role in the host tenant. Cross-tenant shared channels wait for federation.
- **Model:** team → channel (standard, private, direct, group) → message → thread. Messages are append-only events: create, edit, delete, react, read-mark. One log serves sync, audit and retention. Message bodies are a small structured rich-text tree (no HTML on the wire).
- **Storage:** Postgres is the default for Teams (write rate, row-level security, partitioning by channel, full-text search); SQLite (WAL) stays supported for single-user and small installs through the same `sqlx` traits. Attachments are Drive blobs referenced by id, so permissions come from Drive. Multi-node fan-out starts with Postgres `LISTEN/NOTIFY`; add an external broker only when measurements demand it (ADR rule: no infrastructure for its own sake).
- **Realtime: WebSocket** ([RFC 6455](https://www.rfc-editor.org/rfc/rfc6455)) per client. The protocol is: authenticate with a short-lived ticket, subscribe by scope, receive events with a per-scope sequence number, resume from the last sequence after a drop. Mobile apps do not hold background sockets; push wakes them (section 5).
- **Control surface:** every Teams action is a command with an effect class. Sending a message is `egress`, so agents and MCP get confirmation and audit like any other egress.

### Matrix or custom protocol

| | Adopt Matrix as the core | Custom protocol + bridge (chosen) |
|---|---|---|
| Spec and licence | Spec Apache-2.0; `matrix-rust-sdk` Apache-2.0; `vodozemac` Apache-2.0; Ruma MIT; Conduit and Tuwunel Apache-2.0; Synapse and Dendrite AGPL-3.0 (do not link or embed; verify) | Ours (MIT, ADR-008) |
| Fit with ADR-009 | Needs a second server (homeserver) beside ours, or our server becomes a homeserver | One binary |
| Tenancy and permissions | A homeserver is one namespace; per-tenant isolation means one homeserver per tenant, and Drive permissions would have to be mirrored into room power levels | Native: one permission function for files and chat |
| Federation | Built in, proven | Not built; bridge in P4 |
| Presence, typing, read markers | Exist but are known to be costly at scale; verify per homeserver | Designed for our scale target |
| Clients | `matrix-rust-sdk` gives sync, E2EE and storage; mobile bindings use UniFFI (MPL-2.0, flagged in ADR-010) | We write the client core, which we must do for native UIs anyway |
| Calls | MatrixRTC with LiveKit exists | Our `MediaProvider` trait |
| Risk | Protocol shape (room DAG, state resolution) drives our data model; hard to change | Protocol design effort; no federation until the bridge |

Reasons: (1) ADR-009 says self-contained with open standards; a second server breaks that. (2) Tenant isolation and file-permission inheritance are the product's core, and Matrix's room model does not map onto them without a mirror layer that becomes the hard part. (3) Mobile clients will be thin Rust cores; we control size and sync behaviour. (4) Matrix remains attractive for federation, so the event schema uses Matrix-compatible concepts (event types, relations for threads and reactions, stable ids) to keep a bridge cheap. That bridge is a separate component (appservice style) in P4, off by default, with allowlisted rooms.

## 3. Meetings media

An SFU forwards each participant's streams. Mesh does not scale past a few people; an MCU is too costly.

| Option | Licence | Language | Ops cost | Mobile and client SDKs | Notes |
|---|---|---|---|---|---|
| **LiveKit** server | Apache-2.0 | Go | One binary plus built-in TURN; Redis needed only for multi-node; simple small installs | Official Swift, Android, JS and Rust SDKs, all Apache-2.0; E2EE support in SDKs (verify) | Mature, widely deployed; used by Matrix Element Call. A separate process, not our binary |
| mediasoup | ISC | C++ worker + Node or Rust API | We write the signalling and room layer ourselves | Client SDKs are mostly community/JS | Library, not a product; more work than LiveKit |
| str0m | MIT/Apache-2.0 | Rust, sans-IO | We write the whole SFU, congestion control tuning and TURN | None; clients use libwebrtc | Fits single-binary Rust; high effort and risk |
| webrtc-rs | MIT/Apache-2.0 | Rust | As str0m | None | Slower release pace than str0m (verify); not preferred |

**Decision:** ship LiveKit as an optional, operator-run sidecar behind a `MediaProvider` trait. Our server mints short-lived, room-scoped tokens and owns membership, lobby and moderation; LiveKit only moves media. Clients use the LiveKit Rust SDK on desktop (it wraps libwebrtc, BSD-3-Clause), browser WebRTC through `web-sys`, and the Swift and Android SDKs on mobile. In parallel, a str0m spike builds a minimal Rust SFU for one-to-many audio and video. We adopt it only if it meets the T3 quality targets with less operational weight. Evidence beats preference here.

LiveKit is a Go process, so it breaks "one binary" for meetings only. That is an accepted cost: chat, files and calendar run without it, and TURN is bundled.

## 4. End-to-end encryption

| Option | Pros | Cons |
|---|---|---|
| None in v1: TLS plus at-rest encryption, server trusted | Search, retention, eDiscovery, admin recovery, multi-device work simply | Operator can read messages |
| Olm/Megolm (`vodozemac`, Apache-2.0) | Proven in Matrix | Ties us to Matrix key semantics |
| MLS ([RFC 9420](https://www.rfc-editor.org/rfc/rfc9420), `openmls`, MIT) | IETF standard, scales to large groups, forward secrecy | Server-side search and retention need client cooperation; device verification UX is hard |
| Media E2EE (SFrame / insertable streams) | Keeps media private from the SFU | Needs key distribution via the chat layer |

**Decision:** v1 is server-trusted, which matches enterprise suite expectations (retention, audit, search). Opt-in E2EE per channel and per call arrives after T4, using MLS for messages and a shared-key scheme for media. T4 includes a spike. The UI must label E2EE channels clearly, and say which features they disable (server search, retention export).

## 5. Push notifications

Constraint: APNs and FCM credentials belong to the app publisher, not the operator. A self-hosted server cannot push to the official app with its own credentials. Mitigations:

- **Project-run push gateway** (small, open source, optional). The operator's server sends the gateway an opaque device token, tenant id and event id. It forwards a content-free wake-up. The app then fetches the message from its own server. Operators who build and sign their own app can run their own gateway.
- **Android without Google:** [UnifiedPush](https://unifiedpush.org/) support, on Android only.
- iOS: a notification service extension fetches content after the wake-up. Apple's rules on background delivery apply (verify).
- No message text, sender name or channel name leaves the operator's server through the gateway. The gateway sees device tokens and tenant ids; its logs rotate and hold no content.

## 6. Client architecture per platform

One Rust crate set (`open365-teams-core`): protocol client, sync engine with resume, local SQLite cache, command layer, sanitised rich-text model, media-session control. Platform shells (ADR-010) stay thin:

- **Desktop:** our own winit/wgpu UI stack (`adr-ui-platform.md`); media via the LiveKit Rust SDK; capture through Windows Graphics Capture, ScreenCaptureKit and the PipeWire portal.
- **Web:** Rust/WASM driving the DOM; browser WebRTC; `getDisplayMedia` for screen share.
- **iOS:** UIKit over the Rust core; LiveKit Swift SDK; ReplayKit broadcast extension for screen share.
- **Android:** Views over the Rust core; LiveKit Android SDK; `MediaProjection` in a foreground service; FCM or UnifiedPush.

## 7. Interoperability with Microsoft Teams

**v1: none.** The Teams client and federation protocols are not public, and clean-room rules (CLAUDE.md) forbid reverse engineering them or reproducing private behaviour. If demand appears, an optional bridge may use only Microsoft's documented Graph APIs (chat, subscriptions, online meetings), under a tenant admin's consent and an app registration in the customer's own tenant, with every call audited. Nothing joins Microsoft meetings through undocumented signalling. Calendar uses iCalendar ([RFC 5545](https://www.rfc-editor.org/rfc/rfc5545)), iTIP ([RFC 5546](https://www.rfc-editor.org/rfc/rfc5546)) and later CalDAV ([RFC 4791](https://www.rfc-editor.org/rfc/rfc4791)).

## 8. Threat-model delta (proposed rows T17 to T25)

1. **T17 Message content injection:** rich text is a typed tree rendered by native widgets or DOM text nodes; pasted HTML is sanitised; links show their real target.
2. **T18 Subscription authz:** every WebSocket subscribe and resume is checked by the permission function; membership changes drop live subscriptions at once. A test per event type covers cross-tenant and removed-member cases.
3. **T19 Link previews and webhooks:** fetched only through the one egress client from T12 (private ranges blocked after DNS, redirects and size capped).
4. **T20 Metadata leaks:** presence, typing and read receipts are scoped to shared channels; admin policy can disable them; push carries no content.
5. **T21 SFU abuse:** room tokens are single-room, short-lived, identity-bound; the TURN relay denies private and loopback peer addresses (open-relay and SSRF risk); per-room and per-user stream limits; the host can remove and lock.
6. **T22 Capture consent:** screen share requires an explicit picker every time; recording needs host consent and shows an indicator to all; recordings are Drive files with normal ACLs.
7. **T23 File-tab permissions:** a tab grants no access by itself; the document ACL decides per viewer; losing channel membership never widens access.
8. **T24 Retention and legal hold:** message deletion also removes search entries, thumbnails and push queues (extends T7); holds are admin-only and audited.
9. **T25 Abuse and spam:** rate limits per user, channel and tenant; guest limits; audit of bulk mentions and invites.

T6 and T8 apply unchanged and gain a Teams test per route.

## 9. Alternatives considered

Matrix as the core and Tuwunel as the backend (section 2); XMPP (many extensions needed, thin Rust servers); third-party chat SaaS (forbidden by ADR-009); mesh-only WebRTC (fails beyond about 4 participants); building the SFU first (highest risk on the critical path, so it is a spike).

## 10. Confidence and what would change it

- **Custom protocol over Matrix:** medium. Flip if a Rust homeserver passes our tenancy, Drive-permission and mobile-latency tests with less effort than T0 to T2, or if the owner prioritises day-one federation.
- **LiveKit now, str0m as a spike:** medium-high on LiveKit. Flip to a Rust SFU if the T3 spike meets targets with lower operating weight, or if LiveKit's licence, governance or Go runtime becomes a blocker. Flip away from LiveKit if its mobile SDKs fail our accessibility or size budgets.
- **Server-trusted v1:** medium-high. Flip to E2EE-by-default if target customers demand it over search and retention.
- **Push gateway:** medium. Flip if app store policy blocks content-free wake-ups for chat apps.

## 11. Acceptance

Slices T0 to T4 in `../roadmap/first-milestones.md`. Confirmed when T0 passes its tenancy and reconnect tests and the T3 media spike reports numbers.

## Sources (as known 2026-10-11; verify before freezing)

- Matrix specification and licence: https://spec.matrix.org/ and https://github.com/matrix-org/matrix-spec
- matrix-rust-sdk: https://github.com/matrix-org/matrix-rust-sdk (Apache-2.0)
- Conduit: https://gitlab.com/famedly/conduit; Tuwunel: https://github.com/matrix-construct/tuwunel
- Synapse licence change to AGPL-3.0: https://matrix.org/blog/2023/05/synapse-now-agpl/
- LiveKit server and SDKs: https://github.com/livekit/livekit and https://github.com/livekit/rust-sdks (Apache-2.0)
- mediasoup: https://mediasoup.org/ (ISC); str0m: https://github.com/algesten/str0m; webrtc-rs: https://github.com/webrtc-rs/webrtc
- RFC 6455 WebSocket, RFC 9420 MLS, RFC 5545/5546/4791 calendaring: https://www.rfc-editor.org/
- openmls: https://github.com/openmls/openmls (MIT)
- UnifiedPush: https://unifiedpush.org/
- APNs: https://developer.apple.com/documentation/usernotifications; FCM: https://firebase.google.com/docs/cloud-messaging
- Microsoft Graph Teams APIs: https://learn.microsoft.com/en-us/graph/teams-concept-overview
