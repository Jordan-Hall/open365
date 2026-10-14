# Teams landscape: reuse or build

Status: draft for owner review, 2026-10-12. Feeds `../architecture/teams.md` and ADR-016. All "accessed" dates are 2026-10-12 unless stated. Star counts and push dates are snapshots, not quality signals.

Method: licence and activity come from the GitHub REST API (`api.github.com/repos/<owner>/<repo>`: `license.spdx_id`, `pushed_at`, `archived`) and from the project's own README, LICENSE or site. A licence of NOASSERTION means GitHub could not classify the file; the LICENSE text was read instead. Items we could not confirm from a primary source are marked **unverified** and must be re-checked before the ADR is accepted. Licence readings are engineering notes, not legal advice.

## 1. Chat servers and protocols

| Project | Licence (source) | Activity (source, accessed) | Federation | Fit for Open365 |
|---|---|---|---|---|
| **Synapse** (Matrix homeserver, Python + Rust) | AGPL-3.0, or a commercial licence from Element: [README](https://raw.githubusercontent.com/element-hq/synapse/develop/README.rst) | [repo](https://github.com/element-hq/synapse): not archived, last push 2026-10-12. README: "written and maintained by Element"; no support without a subscription | Yes, Matrix server-to-server | Cannot be embedded or linked in an MIT product (ADR-008). Could run as a separate operator-run process only. Python runtime |
| **Dendrite** (Matrix homeserver, Go) | AGPL-3.0: [repo](https://github.com/element-hq/dendrite) | Not archived, last push 2026-09-17 | Yes | Same licence problem. Maintenance cadence is lower than Synapse (last push about 4 weeks before access); we did not read its roadmap, so its feature completeness is **unverified** |
| **Conduit** (Matrix homeserver, Rust) | Apache-2.0: [GitLab](https://gitlab.com/famedly/conduit) | GitLab API `last_activity_at` 2026-10-01 ([API](https://gitlab.com/api/v4/projects/famedly%2Fconduit)); created 2020-10-28 | Matrix federation is the project's purpose; the pages we read did not state completeness | Permissive and Rust. Candidate backend if we ever run Matrix beside our server |
| **conduwuit** (Conduit fork) | Apache-2.0: [repo](https://github.com/girlbossceo/conduwuit) | **Archived**; last push 2026-05-29. Its successor is Tuwunel | n/a | Do not adopt. Dead end |
| **Tuwunel** (successor to conduwuit, Rust) | Apache-2.0: [repo](https://github.com/matrix-construct/tuwunel) | Last push 2026-10-11; latest release v1.9.4 on 2026-10-10 ([releases API](https://api.github.com/repos/matrix-construct/tuwunel/releases/latest)). README: "primarily sponsored by the government of Switzerland", claims "enterprise-ready" and to implement the Matrix spec "for all but the most niche uses" ([README](https://raw.githubusercontent.com/matrix-construct/tuwunel/main/README.md)) | Yes (Matrix) | Best permissive Matrix server today. Claims are self-reported; we have run no tests of our own (**unverified**). A sponsor concentration risk exists |
| **Matrix specification** | Apache-2.0; spec v1.19 ([spec.matrix.org](https://spec.matrix.org/latest/)) | Current | Federation syncs a signed per-room event graph with eventual consistency; optional E2EE with Olm and Megolm | Open and implementable. The room-DAG and state-resolution model is the main design cost to adopt as a core |
| **matrix-rust-sdk** | Apache-2.0: [repo](https://github.com/matrix-org/matrix-rust-sdk) | Not archived, last push 2026-10-12 | Client side | Reusable only if our core speaks Matrix. See ADR-016 |
| **Zulip** | Apache-2.0 ([API](https://api.github.com/repos/zulip/zulip), [docs](https://zulip.readthedocs.io/en/latest/overview/readme.html)) | Not archived, last push 2026-10-12 | The pages we read state no server-to-server federation and no E2EE (absence of a statement, not proof) | Strong topic-threading model worth studying. Python/Django server, so it would be a second stack. Its web client is not our native UI |
| **Mattermost** | Source repo is "Other" (NOASSERTION). [LICENSE.txt](https://raw.githubusercontent.com/mattermost/mattermost/master/LICENSE.txt): compiled builds from Mattermost, Inc. are MIT; self-compiled source is AGPL-3.0 or commercial; `server/public/`, `webapp/`, `server/templates/` and `server/i18n/` are Apache-2.0; trademark needs written approval | Not archived, last push 2026-10-12; latest release v11.11.1 on 2026-09-24 | None found in what we read (**unverified**) | Go server, AGPL core. Cannot embed. Studying its plugin model is fine |
| **Rocket.Chat** | Repo is NOASSERTION. [LICENSE](https://github.com/RocketChat/Rocket.Chat/blob/develop/LICENSE): MIT outside `ee/` and `apps/meteor/ee/`; those directories carry a separate licence whose text we did not read | Not archived, last push 2026-10-12; latest release 8.9.0 on 2026-10-05 | Matrix federation exists as a feature; scope and edition gating **unverified** | Meteor/Node stack, open-core split. Do not adopt: a second stack plus an enterprise carve-out |
| **XMPP** (protocol) | Open standards: RFC 6120, RFC 6121, RFC 7622; XEPs from the XSF ([xmpp.org overview](https://xmpp.org/about/technology-overview/)) | Mature | Email-like federation. Extensions: MUC (XEP-0045), Jingle (XEP-0166/0167) | Many extensions are needed to match Teams features (threads, reactions, file permissions). The overview says only that the community is "actively working on" E2EE |
| **Prosody** (XMPP, Lua) | MIT/X11 ([prosody.im](https://prosody.im/)) | Release 13.0.7 on 2026-09-24 | XMPP federation | Permissive, small. Not a fit for tenancy and Drive permissions |
| **ejabberd** (XMPP, Erlang) | GPL-2.0 with an OpenSSL linking exception ([COPYING](https://raw.githubusercontent.com/processone/ejabberd/master/COPYING)); GitHub reports NOASSERTION | Not archived, last push 2026-10-09 | XMPP federation | GPL: cannot embed in MIT code |

Reading: every mature chat product with the features we need is either copyleft (Synapse, Dendrite, Mattermost source, ejabberd), open-core (Rocket.Chat, Mattermost editions) or a second runtime (Zulip, Rocket.Chat). The only permissive Rust server is Matrix-shaped (Tuwunel, Conduit). None of them has our model of tenant-scoped identity with Drive-permission inheritance.

## 2. Meetings and media

| Project | Licence (source) | Activity | Notes |
|---|---|---|---|
| **LiveKit** server (Go SFU) | Apache-2.0 ([API](https://api.github.com/repos/livekit/livekit)) | Last push 2026-10-12 | Docs state an embedded TURN server and a config-file deployment; Redis is "recommended for production" ([docs](https://docs.livekit.io/home/self-hosting/deployment/)). E2EE: SDKs support enabling it per participant, but key generation, storage and distribution are the application's job ([docs](https://docs.livekit.io/home/client/tracks/encryption/)). Client SDK licences (Swift, Android, Rust) were not re-read here; ADR-014 lists them as Apache-2.0 (**unverified**) |
| **Jitsi Meet** (web app) | Apache-2.0 ([API](https://api.github.com/repos/jitsi/jitsi-meet)) | Last push 2026-10-09 | A complete product (web UI, XMPP signalling). Embeds poorly in a native UI; we would use only its bridge |
| **Jitsi Videobridge** (SFU, Java) | Apache-2.0 ([API](https://api.github.com/repos/jitsi/jitsi-videobridge)) | Last push 2026-10-09 | JVM runtime; signalling is XMPP (Jicofo, Prosody), so a heavier stack than LiveKit |
| **mediasoup** (C++ worker, Node/Rust API) | ISC ([API](https://api.github.com/repos/versatica/mediasoup)) | Last push 2026-10-09 | A library. We would write rooms, signalling, TURN and client SDKs ourselves |
| **str0m** (Rust, sans-IO WebRTC) | MIT ([API](https://api.github.com/repos/algesten/str0m)) | Last push 2026-10-11 | Building block for a Rust SFU, which is a spike (T3); no SFU, no client SDKs |

## 3. End-to-end encryption

| Item | Licence / status (source) | Notes |
|---|---|---|
| **MLS**, [RFC 9420](https://www.rfc-editor.org/rfc/rfc9420) | Proposed Standard (IETF) | Asynchronous group key agreement for groups of two to thousands, with forward secrecy and post-compromise security |
| **OpenMLS** | MIT ([API](https://api.github.com/repos/openmls/openmls)) | Rust, not archived, last push 2026-10-12. Our maturity check (audit status, API stability, delivery-service and ciphersuite support) is **not done**; it is a T7 spike item |
| **Olm/Megolm** (Matrix) | Matrix spec, Apache-2.0 | Only attractive if the core speaks Matrix |
| **Media E2EE** | LiveKit: app-managed keys (see above) | Keys would ride the MLS group. Interoperating SFrame across all four client stacks is unproven |

## 4. Conclusions used by the architecture

1. **No complete product can be embedded** in an MIT codebase under ADR-008 and ADR-009. Synapse, Dendrite and ejabberd are copyleft; Mattermost and Rocket.Chat are open-core; Zulip would add a second stack and has no federation in what we read.
2. **Matrix is the only open federation protocol with a permissive Rust server and client SDK**, so it is the natural federation target. It is a bridge target, not the core (ADR-014 reasons stand; ADR-016 re-tests them).
3. **LiveKit is the only candidate that is permissive, self-hostable with TURN built in and has first-party mobile SDKs** (SDK claim unverified here). Keep it behind `MediaProvider`.
4. **MLS via OpenMLS is the E2EE direction**, validated by a spike before any commitment.
5. Re-check before acceptance: Dendrite and Tuwunel feature completeness, Rocket.Chat `ee/` licence text, LiveKit SDK licences and E2EE interop, OpenMLS audit status, Mattermost and Zulip federation statements.
