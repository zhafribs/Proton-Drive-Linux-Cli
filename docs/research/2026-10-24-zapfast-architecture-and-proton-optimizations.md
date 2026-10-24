# ZapFast vs Proton Drive CLI: what makes the WhatsApp client fast, and what transfers

Research note, 2026-10-24. Sources are primary: the ZapFast repository and its
architecture guide, the protocol library it uses, its published benchmark page,
and this repo's own code and measurements.

## 1. How WhatsApp desktop actually works

- The official WhatsApp Web/Desktop is a web application running inside a
  browser engine (Chromium; Electron-style on desktop). ZapFast's benchmark
  counts **14 Chromium processes** for WhatsApp Web versus **one process** for
  ZapFast, and 1,127.6 MB PSS versus 150.0 MB
  ([benchmarks](https://zapfast.rocks/benchmarks/)).
- A desktop client is a **companion device**: it links by QR code or an
  eight-character pairing code, keeps its own session, receives recent history
  at link time, and can request older messages from the phone on demand. The
  phone does not need to stay online to read what is already stored
  ([ZapFast README](https://github.com/crmne/zapfast)).
- The transport is a persistent WebSocket speaking WhatsApp's own protocol:
  a Noise handshake and certificate trust for the connection, Signal
  encryption for messages (including sender keys for groups), and protobuf
  payloads. Media travels over HTTPS using keys carried in the message
  protobuf. This is visible in the structure of the library ZapFast uses:
  `wacore` owns protocol, crypto, typed IQs and neutral storage/runtime
  traits; `waproto` holds generated protobufs; Noise selection and
  certificate trust live in `handshake.rs`/`wacore/noise`; Signal state lives
  in `wacore/libsignal`
  ([whatsapp-rust architecture](https://github.com/oxidezap/whatsapp-rust/blob/main/agent_docs/architecture.md)).

## 2. What ZapFast is

Rust + [egui](https://www.egui.rs) + [fastframe](https://fastframe.dev), with
[whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) as the protocol
library. One native process, no browser engine, no hosted backend. Chats are
stored in a local SQLCipher SQLite archive; media and avatars are cached per
account; older history is fetched from the phone as you scroll
([README](https://github.com/crmne/zapfast),
[AGENTS.md](https://github.com/crmne/zapfast/blob/main/AGENTS.md),
[Cargo.toml](https://github.com/crmne/zapfast/blob/main/Cargo.toml)).

Measured on one Linux desktop (median of four paired runs, 15 Sep 2026):

| Measurement | ZapFast 0.13.1 | WhatsApp Web + Chromium |
| --- | --- | --- |
| Idle RAM (PSS) | 150.0 MB | 1,127.6 MB |
| Process start to first window | 152 ms | 528 ms |
| Chat UI observed | ~0.69 s | ~4.13 s |

ZapFast 0.16, with in-app video/image previews and animated stickers, sits
around 200 MB idle on the same desktop
([benchmarks](https://zapfast.rocks/benchmarks/)).

## 3. Why it is fast and small — the levers

1. **No browser engine.** One process instead of a browser's browser, GPU,
   renderer, utility and zygote processes.
2. **The protocol runs in-process.** No JavaScript bundle, no Node runtime,
   no per-action subprocess. The library owns pairing, encryption, history
   sync and media.
3. **Storage sized for the data.** SQLCipher SQLite with paged reads, not a
   whole archive in memory. History the user has not scrolled to is not held.
4. **A UI stack chosen for footprint.** `eframe` is built with
   `default-features = false, features = ["glow", ...]` — OpenGL rather than
   wgpu — and the release profile is `lto = "thin"`, `codegen-units = 1`,
   `strip = true`, `panic = "abort"`.
5. **Event-driven, not polling.** A backend worker thread owns the runtime;
   the UI and worker exchange `Command`/`Event` and every event wakes the
   window through a `Waker`. The architecture guide states plainly: "Repaints
   are event-driven, so nothing spins."
6. **Paced work.** Group metadata is queried two at a time per five seconds,
   history requests are serialised with backoff, read-state writes are
   serialised, and paused animations decode only their poster frame. Each is a
   deliberate limit against rate limits and wasted CPU.

## 4. Our app today (measured on this machine, 3.16.18 idle)

| Component | PSS |
| --- | --- |
| `proton-drive-gui` (egui/wgpu) | 127.3 MiB |
| bundled Python engine (`--loop`) | 36.6 MiB |
| AppImage wrapper (engine) | 23.3 MiB |
| AppImage wrapper (GUI) | 18.9 MiB |
| **Total** | **~207 MiB** |

Scan peak was earlier measured at 132.5 MB RSS for the process tree; the
engine spawns one official `proton-drive` CLI process per parallel scan
worker, each roughly 100 MB because the CLI is a Node/SDK process.

So the shape is already closer to ZapFast than to a browser app — native Rust
GUI, one engine process, metadata snapshot, event-driven push/inotify — but
three levers differ.

## 5. Where their levers map onto ours

| Lever | ZapFast | Proton app today | Transferable? |
| --- | --- | --- | --- |
| Browser engine | none | none (egui) | Already there |
| Protocol in-process | whatsapp-rust | official CLI subprocess per folder (~100 MB each) | **No** — Proton has no official public API; reimplementing it is the account-restriction risk we already hit |
| Storage | SQLite, paged, lazy history | one JSON snapshot + JSON state; whole tree in memory | Partly, only for very large trees |
| UI stack | eframe `glow`, no wgpu | `eframe = "0.37.0"` with defaults (wgpu) | **Yes** — concrete, measurable |
| Event-driven | `Waker`, zero idle repaints | health file polled every 3 s; local folder stats re-walked every 7 ticks (~21 s) | **Yes** — inotify on health/state + cached stats |
| Release profile | thin LTO, 1 CGU, strip, panic=abort | full LTO, 1 CGU, strip | Minor: add `panic = "abort"` |
| Paced work | request limits/backoff | adaptive scan workers, RAM cap, rate-limit backoff, per-parent transfer batches, chunked audit | Already aligned |

## 6. Recommendations, ranked

1. **Switch the GUI to eframe's `glow` backend** (`default-features = false,
   features = ["glow", "wayland", "x11", ...]`). It drops the wgpu stack,
   shrinks the binary, and is the most likely source of the 127 MiB GUI PSS.
   Measure PSS and startup before/after; keep wgpu if rendering regresses.
2. **Stop polling the health file and re-walking the local tree on a timer.**
   Watch `health.json` (and `show-request`) with inotify and repaint only on
   change; cache the "files / bytes" totals and refresh them after cycles, not
   every ~21 s. Small CPU/battery win, noticeable with large folders.
3. **Only if the tree grows past ~10k entries:** move the snapshot/state from
   one JSON blob to SQLite (or at least an append-friendly binary format),
   and write audit chunks incrementally instead of rewriting the snapshot.
   At 1,230 entries the current file is not the bottleneck.
4. **Add `panic = "abort"` to the release profile.** Small binary/memory win,
   no behavioural change (the GUI has no unwinding-dependent recovery).
5. **Do not reimplement the Proton protocol.** The WhatsApp client can skip
   the CLI because a mature Rust protocol library exists and the account is
   already at risk either way (Meta explicitly permits the risk to be the
   user's). Proton has no official third-party API, and the earlier attempt
   with the bundled web client coincided with an account restriction. Staying
   on the official CLI is the safety property of this project.

## 7. "Can you build a ZapFast-style WhatsApp app too?"

Yes, in the sense that matters: the hard part already exists as a library.
`whatsapp-rust` provides pairing, Signal/Noise, history sync, media, groups
and calls; a ZapFast-like client is then Rust + egui + a SQLite archive +
that library. An MVP (link, chat list, send/receive text, media download)
is a few weeks of focused work; parity (calls, stickers, history paging,
multi-account, theming, packaging for three OSes) is months, and is exactly
what ZapFast's 847-line architecture guide documents.

From scratch — implementing Noise, Signal double ratchet, sender keys, media
encryption and history sync — is person-years and not something to start.

The caveat is not technical: unofficial WhatsApp clients violate Meta's terms
and can get an account suspended. ZapFast says so in its own README. Building
one is a legitimate learning project; shipping one asks users to accept that
risk.

## Sources

- ZapFast repository: https://github.com/crmne/zapfast
- ZapFast architecture guide: https://github.com/crmne/zapfast/blob/main/AGENTS.md
- ZapFast dependencies and release profile: https://github.com/crmne/zapfast/blob/main/Cargo.toml
- ZapFast benchmark method and data: https://zapfast.rocks/benchmarks/
- whatsapp-rust library: https://github.com/oxidezap/whatsapp-rust
- whatsapp-rust architecture: https://github.com/oxidezap/whatsapp-rust/blob/main/agent_docs/architecture.md
- This repo: `gui/Cargo.toml`, `gui/src/ui/mod.rs`, `scripts/proton-sync.py`,
  and local PSS samples taken 2026-10-24.
