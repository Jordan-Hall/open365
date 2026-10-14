# Open Teams: scope, components and milestones

Status: proposed, 2026-10-12, awaiting owner review (ADR-016). "Open Teams" is a working name; the product name is an owner decision (no Microsoft marks, per the app-UI rule). Evidence for every third-party claim is in `../research/teams-landscape.md`. This document extends ADR-014 (`adr-teams.md`): that file holds the first server, media, push and platform analysis and is not repeated here. Where this document changes ADR-014, section 12 says so.

Separation of facts, assumptions and open points follows CLAUDE.md. Sections 1 to 11 are design (proposal); "Confirmed" appears only where a source or the repository was read.

## 1. Repository starting point (confirmed 2026-10-12)

- `crates/server` (`open365-server`) already has: local accounts, OIDC (feature `oidc`), TOTP, sessions, `TenantCtx`, a single permission function (`perm.rs`), tamper-evident per-tenant audit (`audit.rs`), rate limiting, share links, blob store, SQLite migrations 001 to 003 and a route-table tenant-isolation test (T14 in the threat model). It has no WebSocket, no Postgres backend, no outbound HTTP client except OIDC, no push, and no messaging tables.
- `crates/commands` and `crates/mcp` give the command layer with effect classes and origins. Teams actions join it (section 9).
- Desktop UI is the own winit/wgpu stack (`adr-ui-platform.md`); Writer and Sheets are separate apps (ADR-012). Teams is a third app with the same shell approach.

## 2. Scope at Microsoft Teams parity

Each row is a product capability, its phase slice (section 10) and the main dependency. "Parity" means observable behaviour that users of Teams rely on, measured per slice; no claim of full parity is made without test evidence.

| Capability | Slice | Notes and dependency |
|---|---|---|
| Teams, standard/private channels, 1:1 and group chats | T0, T1 | Tenant-scoped model from ADR-014 |
| Threads and replies, edit, delete, reactions, @mentions, unread counts | T0, T1 | Append-only events |
| Rich text, code blocks, emoji, link previews | T1, T2 | Typed tree, no HTML on the wire; previews via the one egress client |
| Files via our storage (Drive blobs), file tabs opening Writer or Sheets | T2 | Permissions come from the document ACL, never from the channel |
| Presence, typing, read receipts, status message | T2 | Scoped to shared channels; admin can disable |
| Search (messages, files, people) | T2 | Postgres full-text first; permission-filtered |
| Mobile chat (iOS, Android) and push | T3 | Content-free gateway (ADR-014 section 5) |
| 1:1 and group audio/video, join by link, lobby, host controls, in-meeting chat | T4 | `MediaProvider` |
| Screen and window share, recording | T5 | Platform capture APIs; recordings are Drive files |
| Calendar integration, scheduled meetings, notifications | T6 | iCalendar (RFC 5545/5546), CalDAV later (RFC 4791) |
| Admin, retention, legal hold, audit views, eDiscovery export | T6 | Extends T7 in the threat model |
| Apps, bots, webhooks, Open CoWorker | T7 | Scoped app principals; CoWorker is an app principal, not a privileged path |
| Optional E2EE channels and calls; federation bridge | T8 | MLS; Matrix bridge |
| Breakout rooms, live captions, together mode, whiteboard, large events | Not scheduled | Listed so the gap is explicit |

## 3. Components

New crates keep the layering rule (format and protocol crates never depend on UI crates):

| Crate | Role | Depends on |
|---|---|---|
| `open365-teams-proto` | Wire types: events, commands, subscriptions, errors, rich-text tree. Serde + a JSON schema checked in. Versioned (`proto_version` in the handshake). No I/O | none |
| `open365-teams-core` | Client core: WebSocket client, sync with per-scope sequence resume, local SQLite cache, command layer binding, outbox with idempotency keys, media-session control | proto |
| `open365-teams-server` (module set in `crates/server` first; split only when it hurts) | Messaging tables, fan-out, search, presence, retention jobs, push dispatch, media tokens | server, proto |
| `open365-media` | `MediaProvider` trait + LiveKit adapter; token minting, room policy | server |
| `open365-push-gateway` | Optional, separate small binary run by the project or the operator | proto |
| `open365-teams-ffi` | UniFFI or hand-rolled C ABI over core for Swift and Kotlin shells | core |
| `apps/teams` (desktop), `apps/teams-web` (Rust/WASM), iOS and Android shells | Native UI per ADR-010 | core |
| `open365-teams-bridge-matrix` (T8, off by default) | Appservice-style bridge, allowlisted rooms | server |

The CLI and MCP gain `open365 teams ...`; commands carry effect classes (`send` is `egress`, `delete`/`purge` are `destructive`, `export` is `export`).

## 4. Identity and tenancy reuse

- One user directory and one session system: Teams uses the server's accounts, OIDC and TOTP. No second login. A WebSocket opens with a short-lived single-use ticket minted from an authenticated session; the ticket binds the tenant and device.
- `TenantCtx` stays the only source of the tenant. Every new table carries `tenant_id`; routes join the existing route-table isolation test, and every WebSocket subscribe/resume gets the same test (T18).
- One permission function answers both file and chat questions: `can(principal, action, resource)` where a resource is team, channel, message, file, meeting. Channel membership grants chat access only; file access stays on the document ACL (T23).
- Guests are principals with a restricted role inside the host tenant. Cross-tenant shared channels wait for federation (T8).
- Audit uses the existing per-tenant HMAC chain with new event kinds. Message bodies are not written to the audit log by default.

## 5. Reuse versus build

| Subsystem | Decision | Why (evidence) |
|---|---|---|
| Chat server and protocol | **Build** on `open365-server` with an event schema shaped for a Matrix bridge | No embeddable product exists under ADR-008/009 (landscape section 4). Tenancy and Drive-permission inheritance are our core; a Matrix homeserver would need a mirror layer |
| Federation | **Bridge** to Matrix (Tuwunel or Conduit as the peer, or our own appservice) | Matrix is the only open protocol with a permissive Rust stack |
| Meetings media | **Reuse** LiveKit as a sidecar; **spike** a str0m SFU | Apache-2.0, built-in TURN, first-party SDKs |
| E2EE | **Reuse** OpenMLS after a spike | MIT, RFC 9420 |
| Client UI | **Build** (ADR-010/012) | Forbidden to embed web views on desktop |
| Push | **Build** a small gateway; UnifiedPush on Android | Publisher-held APNs/FCM credentials |

**Alternatives and consequences.** (a) Adopt Matrix as the core with Tuwunel: gives federation and E2EE sooner, costs a second server, a tenancy mirror and a protocol shape we do not control. (b) Run Zulip or Mattermost beside the suite: fastest chat, but copyleft or open-core licences, a second stack and no shared permissions. (c) XMPP: mature federation, but many extensions to reach Teams features. Chosen path costs more protocol design and test effort up front and defers federation to T8. **Confidence:** medium. **What would change it:** the same flip conditions as ADR-014, plus new evidence that Tuwunel passes a tenancy and Drive-permission harness (T0 includes the harness definition so the comparison is runnable) or that the owner ranks day-one federation above single-binary hosting.

## 6. Protocol

- Transport: HTTPS for commands and history; one WebSocket per client for events. Auth, subscribe by scope, per-scope monotonic sequence, resume from last sequence, server-sent `gap` forcing a snapshot refetch.
- Events are append-only: `message.create/edit/delete`, `reaction.add/remove`, `read.mark`, `membership.change`, `channel.update`. Every event has a stable id, `tenant_id`, `scope`, `seq`, author principal, client idempotency key and a Matrix-mappable relation (`thread`, `reaction`, `replace`).
- Rich text: typed tree with a closed set of nodes. Unknown nodes degrade to plain text. Size limits per message, per attachment count and per rate.
- Limits and back-pressure are part of the schema (maximum frame size, subscription count per socket).

## 7. Calls, meetings and recording

`MediaProvider` mints room-scoped, short-lived, identity-bound tokens and owns lobby, roles, mute, remove and lock. Screen share always needs a fresh picker. Recording needs host consent and a visible indicator; the recording is a Drive file with an ACL. Dial-in (PSTN/SIP) is out of scope until a demand case exists.

## 8. Federation, E2EE and mobile

- **Federation:** off by default, allowlist-only (T8 in the threat model). Phase one is a Matrix bridge for allowlisted channels, one direction at a time, with per-server rate limits. Open365-to-Open365 server federation follows only after the bridge proves the event mapping. Microsoft Teams interop remains out of scope (ADR-014 section 7).
- **E2EE:** per-channel opt-in using MLS. The UI states what it disables (server search, retention export, bots that cannot hold keys). Media keys ride the MLS group. Spike first: OpenMLS maturity, multi-device, history for new members, and LiveKit key rotation.
- **Mobile:** Rust core + thin native shells (ADR-010). No background sockets; push wakes the app, which fetches over its own connection. Offline outbox with idempotency keys. Size, cold-start and accessibility budgets are measured in T3, not assumed.

## 9. Agent and Open CoWorker surface

All actions are commands with effect classes. Bots and CoWorker are app principals with explicit per-channel grants, rate limits and audit. Agent-origin sends and any file attach need native confirmation unless the user pre-approved that channel. Message text is untrusted data for any model (extends T4 in the threat model). Provider BYOK, budgets and policy remain in the CoWorker design, not in Teams.

## 10. Milestones (small vertical slices)

Durations are estimates for a small team. Numbers in acceptance tests ("stated target") must be fixed with hardware and corpus at the start of the slice, per CLAUDE.md. This table supersedes the T0 to T4 grouping in `../roadmap/first-milestones.md`; the lead should reconcile that file.

| Slice | Demonstrable result | Acceptance test |
|---|---|---|
| **T0** Messaging core (3-4 wk) | `open365 teams send/read/watch` against the server: teams, channels, threads, WebSocket with resume, SQLite and Postgres | Two CLI clients exchange 1,000 messages across three forced disconnects: zero loss, zero duplicates, order preserved per channel. Route-and-subscription test with a second tenant returns 404/denied on every path. Removed member's live subscription closes within 1 s. Agent send is `egress`, audited. Also: a written tenancy and Drive-permission harness that a Matrix server could be run against |
| **T1** Chat client, desktop and web (6-8 wk) | Native desktop and WASM web apps: channel list, threads, edit/delete, reactions, mentions, unread, Drive attachments | Scripted two-user session passes on Windows, Linux, macOS and two browsers. IME and screen-reader announcement tests pass. Pasted HTML is sanitised (corpus of injection payloads). 50k-message channel scrolls within a stated p95 frame time |
| **T2** Presence, file tabs, search (4-6 wk) | Presence/typing/read receipts, file tab opening Writer/Sheets, message and file search | User without document access sees the tab, never the content. Search never returns a channel the user left or a deleted message. Presence invisible outside shared channels |
| **T3** Mobile chat and push (8-10 wk) | iOS and Android chat on the shared core; push gateway | Message reaches a locked phone within a stated latency on both OSes; push payload contains no text, sender or channel name (payload capture test). Offline send then reconnect delivers once. Screen-reader pass on both |
| **T4** Calls and meetings (8-10 wk) | 1:1 and group audio/video, join by link, lobby, host controls, in-meeting chat via `MediaProvider` | 8 participants across desktop, web, iOS, Android hold 30 min within stated loss/latency on stated networks. A room token fails for any other room. Host removal ends that participant's media within 2 s. TURN refuses private and loopback peers (test) |
| **T5** Screen share and recording (6-8 wk) | Screen/window share on every client; recording to Drive | Share starts only through a picker, every time. Recording refuses to start without host consent and shows an indicator to all; the file's ACL equals the meeting's. Stop-share leaves no capture session running (process check) |
| **T6** Calendar, notifications, admin and retention (8-10 wk) | Scheduled meetings with .ics, notification settings, retention policy, legal hold, audit view, export | Meeting round-trips through a standard .ics. A retention rule deletes messages, search entries, thumbnails and push queue items (test per store). A legal hold blocks that delete and is admin-only and audited. Notification mute is honoured by push |
| **T7** Apps, bots, CoWorker (6-8 wk) | App principals with scoped grants, incoming/outgoing webhooks, CoWorker as an app | A bot without a grant cannot read or post. Webhook fetches go through the one egress client (private ranges blocked after DNS). A prompt-injection corpus in messages cannot make a read-only CoWorker send or delete |
| **T8** E2EE and federation spikes then bridge (8-12 wk) | MLS channel behind a flag; allowlisted Matrix bridge | E2EE: server database holds no plaintext for a flagged channel (grep test); a removed member cannot decrypt later messages; UI labels disabled features. Bridge: events map both ways for an allowlisted room; a non-allowlisted room is refused; remote content passes the T1 sanitiser. Spike reports written before any default changes |

## 11. Threat-model additions

Rows T17 to T25 (proposed in ADR-014) are carried into `../security/threat-model.md` with this document's additions T26 to T30: recording consent and storage, bot and app principals, E2EE key and membership handling, retention across derived stores, and bridge trust. Each slice's acceptance test above maps to at least one row.

## 12. Differences from ADR-014

1. Slice numbering extends to T8 (calendar and governance split from screen share; bots and E2EE/federation get their own slices; search moved earlier than mobile).
2. Matrix is re-tested with verified sources: Tuwunel is a viable permissive server and conduwuit is archived, so the "Tuwunel as backend" alternative is kept alive by the T0 harness rather than dismissed.
3. Three claims ADR-014 marked "verify" are still unverified: LiveKit SDK licences and E2EE interop, Rocket.Chat `ee/` terms, OpenMLS audit status. They block acceptance of the ADR, not the T0 start.

## 13. Open questions for the owner

1. Product name for the Teams app (not "Teams").
2. Day-one federation versus single-binary simplicity (flips ADR-016).
3. Is server-trusted chat acceptable for v1 with E2EE opt-in at T8?
4. Who runs the push gateway for official builds, and is a project-run service acceptable to host?
5. Postgres as the default for Teams: acceptable operational cost for small installs, or SQLite-only until measured?
6. Is PSTN/SIP dial-in required for parity claims?
