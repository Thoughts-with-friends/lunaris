# 18. Wi-Fi and Local Multiplayer

The largest and least finished subsystem. This chapter covers the DS Wi-Fi
hardware registers, the local-multiplayer (MP) protocol built on top, and the
transport abstraction that lets two `lunaris` instances play together.

GBATEK reference:
[DS wireless communications](https://problemkaputt.de/gbatek.htm#dswirelesscommunications)

Reference implementation: melonDS `src/Wifi.cpp` and `src/net/` (vendored for
comparison at `docs/design/melonds/`).

---

## 18.1 Two different things called "Wi-Fi"

The naming here trips everyone up, so it is worth pinning down first
([net.rs:28-31](core/src/hw/net.rs#L28-L31)):

```text
   crate::hw::net::wifi        ── INTERNET play (melonDS's Net.cpp)
                                  emulated Ethernet frames to a real network

   crate::hw::wifi (in net/)   ── the DS Wi-Fi HARDWARE registers
                                  W_* at 4800000h..4808FFFh
```

Both live under [`core/src/hw/net/`](core/src/hw/net/):

```text
   core/src/hw/net/
   ├── mp_interface.rs   backend-agnostic MP frame transport
   │                     (melonDS MPInterface.{h,cpp})
   ├── local/            local wireless between instances in one process
   │   ├── local_mp.rs   (melonDS LocalMP.cpp)
   │   └── semaphore.rs  stdlib has no counting semaphore
   ├── bridge.rs         adapts MpInterface → the hardware's MpTransport
   └── wifi/             the W_ registers + internet path
       ├── mod.rs        register file, tick, power, MP state machine
       ├── regs.rs       every W_ constant
       ├── tx.rs  rx.rs  transmit / receive rings
       ├── mp.rs         MpTransport trait, LinkHints
       ├── bb_rf.rs      baseband / RF chip stubs
       ├── diag.rs       handshake counters
       ├── net.rs  net_driver.rs  packet_dispatcher.rs   internet path
```

### The socket boundary

```text
   nds-core owns NO sockets.
   ───────────────────────────────────────────────────────────────
   in core:      the hardware, the MP protocol, the frame formats,
                 an in-process loopback transport
   in gui/net:   enet room hosting, discovery, the wire protocol
   not ported:   melonDS Netplay.cpp (savestate-synchronised netplay)
                 Net_PCap.cpp / Net_Slirp.cpp (C library bindings)
```

This is stated as a hard constraint in both modules
([net.rs:14-27](core/src/hw/net.rs#L14-L27)) and it is what keeps `nds-core`
testable without a network.

---

## 18.2 The hardware register file

```text
   4800000h  W_ID              chip identifier (per console revision!)
   4800004h  W_ModeReset       hardware reset / mode enable
   4800006h  W_ModeWEP         operating mode (0-4) + WEP mode
   4800010h  W_IF              interrupt flags   ← write-1-to-clear
   4800012h  W_IE              interrupt enable
   4800018h  W_MACAddr0..2     this console's MAC
   4800020h  W_BSSID0..2       the network's BSSID
   4800028h  W_AIDLow          association ID (1..15) — set on assoc success
   480002Ah  W_AIDFull
   4800030h  W_RXCnt           RX engine control
   4800036h  W_PowerUS         power-down control
   480003Ch  W_PowerState
   4800050h  W_RXBufBegin      ─┐
   4800052h  W_RXBufEnd         │  the RX ring, in the 8 KB
   4800054h  W_RXBufWriteCursor │  shared Wi-Fi RAM at 4804000h
   4800056h  W_RXBufWriteAddr   │
   4800058h  W_RXBufReadAddr    │
   480005Ah  W_RXBufReadCursor ─┘
   ...
   4804000h  8 KB Wi-Fi RAM (TX slots + RX ring live here)
```

([regs.rs:22-51](core/src/hw/net/wifi/regs.rs#L22-L51))

Three traps this implementation calls out explicitly
([wifi/mod.rs:29-37](core/src/hw/net/wifi/mod.rs#L29-L37)):

```text
   1. 16-bit register access semantics
      Some W_ registers must be read/written as halfwords; treating one as a
      byte pair silently corrupts a cursor.

   2. IRQ edge-triggering
      W_IF is write-1-to-clear like the main IF, but the *sources* are edge
      triggered — re-asserting a level does not re-fire.

   3. Per-instance MAC uniqueness
      Two instances with the same MAC never associate. The frontend must
      customise the MAC per console.

   4. Non-zero synthetic RF channel tables
      A driver reading an all-zero RF calibration table refuses to enable
      the radio at all.
```

`W_ID` is sourced from the firmware image, not hardcoded
([spi.rs:95-102](core/src/hw/spi.rs#L95-L102), Chapter 16) — the DS driver
checks it against the console revision.

---

## 18.3 The 8-microsecond tick

Wi-Fi is the only peripheral with its own free-running clock event
([wifi/mod.rs:316](core/src/hw/net/wifi/mod.rs#L316),
[wifi/mod.rs:965-974](core/src/hw/net/wifi/mod.rs#L965-L974)):

```rust
    const TIMER_INTERVAL_US: u64 = 8;
```

```rust
        let cycles = crate::nds::NDS::CLOCK_RATE as i64 * Self::TIMER_INTERVAL_US as i64;
        let cycles = cycles - self.timer_error;
        let delay = (cycles + 999_999) / 1_000_000;
        self.timer_error = delay * 1_000_000 - cycles;
        scheduler.schedule(Event::Wifi, HW::on_wifi_timer, delay.max(1) as usize);
```

```text
   Why the error term?

   8 µs at 33.513982 MHz = 268.11… cycles — not an integer.

   Naive rounding to 268:  0.11 cycles lost every tick
                         = 13,750 cycles lost per second
                         ≈ 0.04% clock drift
   Over a multi-minute session two consoles' timestamps diverge by
   milliseconds, which is enough to break MP frame ordering.

   timer_error carries the fractional remainder forward, so the average
   period is exactly right.
```

The tick drives beacons, the MP command/reply cycle, and RX polling
([wifi/mod.rs:1092-1095](core/src/hw/net/wifi/mod.rs#L1092-L1095)):

```rust
    pub(crate) fn tick(&mut self, scheduler: &mut Scheduler, request: &mut InterruptRequest) {
        self.us_timestamp += Self::TIMER_INTERVAL_US;
```

Power-off removes the event entirely rather than leaving it spinning
([wifi/mod.rs:934-944](core/src/hw/net/wifi/mod.rs#L934-L944)):

```rust
        } else {
            scheduler.remove(Event::Wifi);
            if let Some(t) = self.transport.as_mut() {
                t.end();
            }
        }
```

---

## 18.4 Local multiplayer: the protocol

DS local play is **not** peer-to-peer chat. It is a strictly host-driven
round-robin, and its shape is what all the machinery below exists to serve.

```text
   HOST                                          CLIENTS (up to 15)
   ────                                          ──────────────────
   beacon ──────────────────────────────────────►  scan finds the room
                                              ◄──  auth request
   auth response ───────────────────────────────►
                                              ◄──  assoc request
   assoc response (grants AID 1..15) ───────────►
                                                   now associated
   ══════════════ per MP frame, repeatedly ══════════════

   CMD frame (TX slot 1) ───────────────────────►  all clients
                                              ◄──  reply (AID 1)
                                              ◄──  reply (AID 2)
                                              ◄──  reply (AID 3)
   ACK frame (+ run-ahead window) ──────────────►

   The host will not send the next CMD until it has collected replies
   (or timed out). That is why MP play is lock-step, and why one slow
   instance stalls everybody.
```

The `MpTransport` trait mirrors that structure exactly
([wifi/mp.rs:85-120](core/src/hw/net/wifi/mp.rs#L85-L120)):

```rust
pub trait MpTransport: Send {
    /// Called when Wi-Fi hardware power turns on ...
    fn begin(&mut self);
    fn end(&mut self);

    /// Broadcasts a regular MP data/beacon/auth/assoc frame to all
    /// MP-ready peers. Returns the number of bytes accepted.
    fn send_packet(&mut self, data: &[u8], timestamp_us: u64) -> usize;

    /// Broadcasts a host MP command frame (TX slot 1).
    fn send_cmd(&mut self, data: &[u8], timestamp_us: u64) -> usize;

    /// Unicasts a client MP reply frame to the host.
    fn send_reply(&mut self, data: &[u8], timestamp_us: u64, aid: u16) -> usize;

    /// Broadcasts a host MP acknowledgement frame, carrying the run-ahead
    /// window granted to clients.
    fn send_ack(&mut self, data: &[u8], timestamp_us: u64, runahead_us: u32) -> usize;

    /// Non-blocking poll for any inbound frame (regular RX path).
    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv;

    /// Bounded blocking wait for the next frame from the host. Used by MP
    /// clients at their sync point ...
    fn recv_host_packet(&mut self, buf: &mut [u8], timeout: Duration) -> MpRecv;

    /// Host-only: collects reply frames from the clients named in `aid_mask`.
    /// Returns the bitmask of AIDs that replied.
```

The doc comment on `recv_replies` is worth reading in full
([wifi/mp.rs:118-126](core/src/hw/net/wifi/mp.rs#L118-L126)):

```rust
    /// # Contract
    /// Every implementation must obey all of the following. There are three
    /// implementations in this workspace and they diverged once already, which
    /// is what let the headless harness report success while real play failed
```

A trait with three implementations and a subtle contract is exactly where a
test harness passes while reality fails. Writing the contract into the trait —
not the design doc — is the mitigation.

---

## 18.5 The stack, layer by layer

Before the sequence diagrams, this is who calls whom. Every arrow crossing a
box boundary is a place a connection can silently fail.

```text
 ┌──────────────────────────────────────────────────────────────────────────┐
 │ GAME (ARM9)                                                              │
 │   "open a Union Room" / "start a 4-player race"                          │
 └────────────────────────────────┬─────────────────────────────────────────┘
                                  │ IPC FIFO (Chapter 7)
 ┌────────────────────────────────▼─────────────────────────────────────────┐
 │ WIRELESS DRIVER (ARM7, from the game's own binary)                       │
 │   programs BB + RF, sets W_ModeWEP, builds 802.11 frames in Wi-Fi RAM    │
 └────────────────────────────────┬─────────────────────────────────────────┘
                                  │ MMIO reads/writes  4800000h..4808FFFh
 ┌────────────────────────────────▼─────────────────────────────────────────┐
 │ Wifi   (core/src/hw/net/wifi/mod.rs)                                     │
 │   register file · TX slots · RX ring · MP state machine                  │
 │   driven by Event::Wifi every 8 µs                                       │
 └────────────────────────────────┬─────────────────────────────────────────┘
                                  │ trait MpTransport
                                  │   send_packet / send_cmd / send_reply /
                                  │   send_ack / recv_packet /
                                  │   recv_host_packet / recv_replies
 ┌────────────────────────────────▼─────────────────────────────────────────┐
 │ transport implementation — ONE of:                                       │
 │                                                                          │
 │   LoopbackTransport      in-core, for tests                              │
 │   MpInterfaceTransport   bridge.rs → MpInterface                         │
 │        └── LocalMp ──► LocalMpHub   (two instances, one process)         │
 │        └── (frontend) ──► lunaris_net  (two machines, one LAN)           │
 └──────────────────────────────────────────────────────────────────────────┘
```

The transport is **installed by the frontend**, not created by the core
([nds.rs:88-94](core/src/nds.rs#L88-L94)):

```rust
    #[inline]
    pub fn set_mp_transport(&mut self, transport: Option<Box<dyn MpTransport>>) {
        self.hw.set_mp_transport(transport);
    }
```

`transport.is_none()` is therefore a perfectly valid state, and it is the first
thing `diag_snapshot` reports (§18.13). A game can enter MP mode, build frames,
and transmit into nothing at all without any error being raised — exactly what
hardware with no other console in range does.

---

## 18.6 Sequence: powering the radio on

```text
 ARM7 driver          Wifi (mod.rs)         Scheduler          MpTransport
      │                     │                    │                   │
      │ write POWCNT2 bit 1 │                    │                   │
      ├────────────────────►│                    │                   │
      │ write W_PowerUS = 0 │                    │                   │
      ├────────────────────►│                    │                   │
      │                     │ power on?          │                   │
      │                     ├─ yes ─────────────►│ schedule(Wifi, 8µs)
      │                     ├─ transport.begin() ┼──────────────────►│
      │                     │                    │                   │
      │ upload BB table     │                    │                   │
      │  (W_BBCnt/W_BBWrite)│  diag.bb_writes++  │                   │
      ├────────────────────►│                    │                   │
      │                     │                    │                   │
      │ upload RF block     │                    │                   │
      │  (W_RFData2 …)      │  diag.rf_transfers++                   │
      ├────────────────────►│                    │                   │
      │                     │                    │                   │
      │ select channel      │  change_channel():                     │
      ├────────────────────►│   look the two RF register values up   │
      │                     │   in the FIRMWARE channel table        │
      │                     │   ├─ found → cur_channel = N           │
      │                     │   └─ not   → cur_channel stays 0  ✗    │
      │                     │                    │                   │
      │ W_ModeWEP = mode    │  is_mp / is_mp_client set here         │
      ├────────────────────►│                    │                   │
      │ W_ModeReset bit 14  │  install RX/filter defaults            │
      ├────────────────────►│  diag.mode_reset++ │                   │
      │ W_RXBufBegin/End    │  diag.rxbuf_cfg++  │                   │
      ├────────────────────►│                    │                   │
      │                     │                    │                   │
      │              radio is now live: tick() runs every 8 µs       │
```

Three ordering facts fall out of this:

- **The BB table is uploaded before the RF chip.** `bb_writes == 0` therefore
  places a failure _earlier_ than `rf_transfers == 0` does.
- **Channel selection is a table lookup against the firmware image**, not a
  register write that always succeeds. A firmware whose calibration block is
  all zeroes leaves `cur_channel == 0` forever, and nothing else in the stack
  will complain (Chapter 16, §16.2 supplies that block).
- **`transport.begin()` fires on power-up, not on room creation.** A transport
  installed after the radio is already on never sees its `begin`.

---

## 18.7 Sequence: host creates a room, client joins

This is the full association handshake. The left column is the host console,
the right the client; both are separate `NDS` instances.

```text
  HOST                          transport / hub                    CLIENT
   │                                   │                              │
   │  ── beacon interval elapses (tick) ──                            │
   │                                   │                              │
   │ build beacon in TX slot           │                              │
   │ send_packet(beacon, ts) ─────────►│                              │
   │   diag.beacon_tx++                ├── push into packet FIFO      │
   │                                   ├── post semaphore[client]     │
   │                                   │                              │
   │                                   │      recv_packet(buf) ◄──────┤ scan
   │                                   ├─ Frame{kind:Packet, ts} ────►│
   │                                   │      diag.rxflags_beacon++   │
   │                                   │                              │
   │                                   │   HOLD until own clock ≥ ts  │
   │                                   │   (§18.11 — this is where a  │
   │                                   │    mismatched epoch kills it)│
   │                                   │                              │
   │                                   │      write frame into RX ring│
   │                                   │      raise W_IF RX bit       │
   │                                   │      driver reads the beacon │
   │                                   │      ── room appears in list │
   │                                   │                              │
   │                                   │◄───── send_packet(AUTH REQ) ─┤
   │◄─ Frame{Packet, mgmt subtype 11} ─┤   diag.rx_mgmt_subtype[11]++ │
   │  diag.last_auth = [...]           │                              │
   │                                   │                              │
   │ send_packet(AUTH RESP) ──────────►│                              │
   │                                   ├─ Frame ─────────────────────►│
   │                                   │                              │
   │                                   │◄───── send_packet(ASSOC REQ)─┤
   │◄─ Frame{Packet, mgmt subtype 0} ──┤                              │
   │                                   │                              │
   │ pick a free AID (1..15)           │                              │
   │ send_packet(ASSOC RESP, aid) ────►│                              │
   │   diag.last_assoc_aid = aid       ├─ Frame ─────────────────────►│
   │   diag.last_assoc_mac_good        │      driver writes W_AIDLow  │
   │                                   │      ── ASSOCIATED ──        │
   │                                   │                              │
   │ ═════════════ from here on, the MP frame cycle (§18.8) ══════════│
```

Two of those steps carry dedicated diagnostic counters precisely because they
are the two that silently fail:

```text
   diag.rx_mgmt_subtype[11] == 0   the host never saw an auth request
                                   → the client never saw the beacon
                                   → beacon_tx? channel? clock epoch?

   diag.last_assoc_aid == 0        assoc response was built but the client
   but client's W_AIDLow == 0      never wrote W_AIDLow back
                                   → set LUNARIS_MP_ASSOC_TRACE: is the driver
                                     reading back the bytes we wrote?
```

The second one is what `set_assoc_trace`
([wifi/mod.rs:80-101](core/src/hw/net/wifi/mod.rs#L80-L101)) exists to answer —
the comment there states the question in exactly those terms.

---

## 18.8 Sequence: the MP frame cycle

Once associated, play is a strict host-driven round. **Nothing is
peer-to-peer.** This is the single most important structural fact about DS
local play, and it is why one slow instance stalls everyone.

```text
   HOST                       transport                  CLIENT 1   CLIENT 2
    │                             │                          │          │
    │ game writes MP payload      │                          │          │
    │ into TX slot 1              │                          │          │
    │                             │                          │          │
    │ send_cmd(data, ts) ────────►│                          │          │
    │   diag.cmd_tx++             ├── broadcast ────────────►│          │
    │                             ├── broadcast ─────────────┼─────────►│
    │                             │  diag.rxflags_cmd++      │          │
    │                             │                          │          │
    │                             │      each client's game reads the   │
    │                             │      CMD, computes its own state,   │
    │                             │      writes its reply payload       │
    │                             │                          │          │
    │                             │◄── send_reply(d, ts, 1) ─┤          │
    │                             │  diag.reply_tx++         │          │
    │                             │◄── send_reply(d, ts, 2) ─┼──────────┤
    │                             │                          │          │
    │ recv_replies(buf, aid_mask, │                          │          │
    │              timeout) ─────►│                          │          │
    │◄── bitmask of AIDs that replied                        │          │
    │   diag.replies_answered++   │                          │          │
    │   (or replies_empty++)      │                          │          │
    │                             │                          │          │
    │ send_ack(data, ts,          │                          │          │
    │          runahead_us) ─────►│                          │          │
    │                             ├── broadcast ────────────►│          │
    │                             ├── broadcast ─────────────┼─────────►│
    │                             │  diag.rxflags_ack++      │          │
    │                             │                          │          │
    │                             │      clients may now run ahead      │
    │                             │      up to runahead_us before their │
    │                             │      next mandatory sync point      │
    │                             │                          │          │
    │ ◄──────────────── next CMD, repeat ───────────────────────────────│
```

The four frame categories are exactly the four `MpTransport` send methods
([wifi/mp.rs:23-32](core/src/hw/net/wifi/mp.rs#L23-L32)):

```rust
pub enum MpFrameKind {
    /// Regular data/beacon/auth/association/deauth traffic.
    Packet,
    /// Host multiplayer command frame.
    Cmd,
    /// Client multiplayer reply frame.
    Reply,
    /// Host multiplayer acknowledgement frame.
    Ack,
}
```

### Reply slots

A reply carries its AID, and the host writes each into a **fixed slot** of the
receive buffer keyed by that AID — the caller does not have to sort them:

```text
   recv_replies buffer, 1024 bytes per client

   offset      0     1024    2048    3072   ...   15×1024
              ┌──────┬──────┬──────┬──────┬─────┬────────┐
              │ AID1 │ AID2 │ AID3 │ AID4 │ ... │ AID15  │
              └──────┴──────┴──────┴──────┴─────┴────────┘
                 ▲
                 └─ data[(aid − 1) * REPLY_SLOT_SIZE]

   returns a bitmask:  0b0000_0000_0000_0110  → AID 2 and AID 3 replied,
                                                AID 1 timed out
```

`aid == 0` would index `(0 − 1) * 1024`. In C that is a wild pointer; the
Lunaris port rejects it outright (§18.12).

### The blank reply

A client with nothing to say still has to answer, or the host waits out its
timeout every round. `diag.blank_reply_tx` counts those
([diag.rs](core/src/hw/net/wifi/diag.rs)), which distinguishes "the client is
silent" from "the client is participating but idle".

### The timing budget

```text
   ├──── one MP round ────────────────────────────────────────────┤
   │                                                              │
   CMD ──► clients compute ──► replies ──► host collects ──► ACK ──►
   │       │                              │                       │
   │       │◄─ recv_host_packet blocks ──►│                       │
   │       │   up to LinkHints::recv_timeout (default 8 ms)        │
   │                                                              │
   │◄──────────── clients may run ahead runahead_us ─────────────►│
   │             (default 1000 µs) past this point                │
```

```rust
impl Default for LinkHints {
    fn default() -> Self {
        LinkHints { runahead_us: 1000, recv_timeout: Duration::from_millis(8) }
    }
}
```

([wifi/mp.rs:67-71](core/src/hw/net/wifi/mp.rs#L67-L71))

Those two numbers are the whole latency/stability trade: a larger `runahead_us`
lets clients stay smooth over a jittery link at the cost of reacting later to
the host; a larger `recv_timeout` tolerates a slow peer at the cost of stalling
everyone when one drops.

---

## 18.9 Sequence: inside the hardware, TX and RX

The diagrams above stop at `MpTransport`. Below it, one frame's journey through
the emulated Wi-Fi hardware looks like this.

### Transmit

```text
   ARM7 driver                     Wifi                         transport
        │                            │                              │
        │ write frame bytes into     │                              │
        │ Wi-Fi RAM (4804000h+)      │                              │
        ├───────────────────────────►│                              │
        │                            │                              │
        │ write W_TXBufLoc[slot]     │  latch slot address + length │
        ├───────────────────────────►│                              │
        │ set W_TXReqSet bit         │  mark slot armed             │
        ├───────────────────────────►│                              │
        │                            │                              │
        │            ── next Event::Wifi tick ──                    │
        │                            │                              │
        │                            │ tick(): a slot is armed?     │
        │                            │   read the frame out of      │
        │                            │   Wi-Fi RAM                  │
        │                            │   classify by slot:          │
        │                            │     slot 1 → send_cmd        │
        │                            │     reply  → send_reply      │
        │                            │     else   → send_packet     │
        │                            ├─────────────────────────────►│
        │                            │   diag.loc_tx++              │
        │                            │                              │
        │                            │ write TX status back         │
        │◄─ W_IF TX-complete bit ────┤ into the slot header         │
        │                            │                              │
```

### Receive

```text
   transport                     Wifi                        ARM7 driver
        │                          │                              │
        │      ── next Event::Wifi tick ──                        │
        │                          │ recv_packet(buf)             │
        │◄─────────────────────────┤  diag.rx_polls++             │
        │                          │                              │
        ├─ MpRecv::None ──────────►│  diag.rx_empty++, done       │
        │                          │                              │
        ├─ MpRecv::Frame{ts, ...} ►│                              │
        │                          │                              │
        │                          │ ts > our clock?              │
        │                          │   → hold as PendingRxHeader, │
        │                          │     re-check next tick       │
        │                          │                              │
        │                          │ filter: BSSID / MAC / mode?  │
        │                          │   → drop, diag.drops.*++     │
        │                          │                              │
        │                          │ accepted:                    │
        │                          │  diag.rx_accepted++          │
        │                          │  build the 12-byte RX header │
        │                          │  copy header + frame into    │
        │                          │  the ring at W_RXBufWriteAddr│
        │                          │  advance W_RXBufWriteCursor  │
        │                          │  raise the W_IF RX bit ──────┤
        │                          │                              │
        │                          │      driver reads through    │
        │                          │      W_RXBufReadAddr, then   │
        │                          │      advances W_RXBufReadCursor
```

```text
   The RX ring, in Wi-Fi RAM
   ┌──────────────────────────────────────────────────────────────┐
   │ W_RXBufBegin                                    W_RXBufEnd   │
   │  ▼                                                        ▼  │
   │  ┌────────┬────────┬────────┬────────┬─────────────────────┐ │
   │  │ hdr+f  │ hdr+f  │ hdr+f  │        │                     │ │
   │  └────────┴────────┴────────┴────────┴─────────────────────┘ │
   │       ▲                     ▲                                │
   │  ReadCursor            WriteCursor                           │
   │  (driver consumed       (hardware produced up to here)       │
   │   up to here)                                                │
   │                                                              │
   │  Wraps at W_RXBufEnd. If WriteCursor catches ReadCursor,     │
   │  the frame is dropped — diag.drops records which reason.     │
   └──────────────────────────────────────────────────────────────┘
```

Two register-level traps live in this diagram, both listed in §18.2:
`W_RXBufWriteCursor` must be written as a **halfword** (a byte-pair write
corrupts it), and the RX interrupt is **edge**-triggered, so re-asserting the
same level does not re-fire it.

---

## 18.10 Sequence: a failed connection, read off the counters

This is the diagram to keep beside you when a room never appears. Each `✗` is
a specific counter reading zero, and each maps to one step of §18.6/§18.7.

```text
   step                                 counter that proves it happened
   ─────────────────────────────────    ────────────────────────────────
   frontend installed a transport   ──► transport_installed
        │ ✗ → the frontend never called set_mp_transport
        ▼
   driver uploaded the BB table     ──► bb_writes > 0
        │ ✗ → the game never started its wireless driver at all
        ▼
   driver programmed the RF chip    ──► rf_transfers > 0
        │ ✗ → driver stopped between BB and RF
        ▼
   a channel was resolved           ──► channel != 0
        │ ✗ → rf_table_empty?        → the firmware image's Wi-Fi block
        │      rf_at_initial_values? → the radio was only re-initialised
        │      neither?              → the game asked for a channel this
        │                              firmware's table does not contain
        ▼
   the game entered MP mode         ──► is_mp
        │ ✗ → the game is scanning, not hosting/joining yet
        ▼
   beacons went out (host)          ──► beacon_tx > 0
        │ ✗ → MP mode set but the beacon interval never elapsed
        ▼
   frames came back (client)        ──► rx_accepted > 0
        │ ✗ but rx_polls > 0 and rx_empty == rx_polls
        │      → nothing on the wire: clock epoch? channel? transport?
        │ ✗ but drops.* > 0
        │      → frames arrive and are filtered out: BSSID/MAC/mode
        ▼
   auth request seen (host)         ──► rx_mgmt_subtype[11] > 0
        ▼
   assoc granted                    ──► last_assoc_aid != 0
        ▼
   client accepted the AID          ──► aid (W_AIDLow) in 1..=15
        │ ✗ → set LUNARIS_MP_ASSOC_TRACE (§18.7)
        ▼
   MP rounds are running            ──► cmd_tx and replies_answered rising
```

That ladder is the reason `MpDiag` carries thirty-odd fields rather than a
boolean: a wireless failure is never "it did not work", it is always "it got
this far and no further", and only the counter that first reads zero tells you
where to look.

---

## 18.11 The wireless timebase

Every MP frame carries a microsecond timestamp, and a receiver **holds a frame
back until its own clock reaches it**. Two consoles started at different
moments therefore cannot talk ([nds.rs:126-147](core/src/nds.rs#L126-L147)):

```rust
    /// Wi-Fi frames carry a microsecond timestamp, and a receiver holds a
    /// frame back until its own clock reaches it. Two consoles that started at
    /// different moments — a second instance opened mid-session — therefore
    /// have to be told about each other, or each reads the other's traffic as
    /// arriving from the future (or the distant past) and the two never
    /// associate.
    #[inline]
    pub fn wifi_clock_reference(&self) -> u64 {
        self.hw.wifi_clock_reference()
    }

    /// Put this console on `us` as its wireless timebase. Call it on the
    /// console that is joining, with [`Self::wifi_clock_reference`] taken from
    /// the one already running, before either turns its radio on.
    #[inline]
    pub fn set_wifi_clock_epoch(&mut self, us: u64) {
        self.hw.set_wifi_clock_epoch(us);
    }
```

```text
   console A started at t=0        console B opened 45 s later
   ──────────────────────────      ────────────────────────────
   uptime_us = 45_000_000          uptime_us = 0

   without an epoch:
     A's beacon says t=45.0 s  →  B thinks it arrived from the future,
                                  holds it forever. No room ever appears.

   with set_wifi_clock_epoch:
     B.clock_epoch_us = 45_000_000
     B's clock = 45_000_000 + its own uptime  →  the two agree
```

The reference is available whether or not the radio is on
([wifi/mod.rs:952-956](core/src/hw/net/wifi/mod.rs#L952-L956)):

```rust
    pub(crate) fn clock_reference(&self, scheduler: &Scheduler) -> u64 {
        self.clock_epoch_us + Self::uptime_us(scheduler)
    }
```

and uptime is derived from the scheduler's master-cycle counter, not from wall
time ([wifi/mod.rs:946-950](core/src/hw/net/wifi/mod.rs#L946-L950)):

```rust
    fn uptime_us(scheduler: &Scheduler) -> u64 {
        (scheduler.cycle as u64).saturating_mul(1_000_000) / crate::nds::NDS::CLOCK_RATE as u64
    }
```

That is deliberate: it means fast-forwarding one instance also fast-forwards
its wireless clock, keeping the two consistent.

---

## 18.12 The local backend

Two consoles in one process talk through a pair of ring FIFOs plus a semaphore
pool ([local/local_mp.rs:6-13](core/src/hw/net/local/local_mp.rs#L6-L13)):

```text
   LocalMpHub  (Arc, shared by every instance)
   ┌──────────────────────────────────────────────────────────────┐
   │  packet FIFO  64 KB ring   regular / CMD / ACK frames        │
   │     write cursor: one, shared                                │
   │     read cursor:  one PER INSTANCE                           │
   │                                                              │
   │  reply FIFO   64 KB ring   client replies                    │
   │     same cursor arrangement                                  │
   │                                                              │
   │  semaphores[0..15]   "instance i has a frame to read"        │
   │  semaphores[16..31]  "instance i has a reply to read"        │
   └──────────────────────────────────────────────────────────────┘
        ▲                          ▲
        │                          │
   LocalMp handle             LocalMp handle
   (instance 0)               (instance 1)
```

```rust
pub const PACKET_QUEUE_SIZE: usize = 0x1_0000;
pub const REPLY_QUEUE_SIZE: usize = 0x1_0000;
pub const MAX_FRAME_SIZE: usize = 0x948;
```

([local/local_mp.rs:47-53](core/src/hw/net/local/local_mp.rs#L47-L53))

### Deliberate deviations from melonDS

Every one of them is a bounds check melonDS achieves by convention
([local/local_mp.rs:23-33](core/src/hw/net/local/local_mp.rs#L23-L33)):

```rust
//! # Deliberate deviations
//! Every one of these is a case where melonDS relies on a raw pointer
//! staying in bounds by convention:
//!
//! * A header whose `length` exceeds [`MAX_FRAME_SIZE`] is treated as FIFO
//!   corruption (resynchronise and drop) instead of being trusted.
//! * `recv_replies` writes reply *aid* at `data[(aid - 1) * 1024]`, as in
//!   melonDS, but rejects `aid == 0` (which would underflow) and `aid > 15`,
//!   and clips a slot that would run past the caller's buffer.
//! * Receive calls copy at most `data.len()` bytes into the caller's
//!   buffer, while still consuming the whole frame from the FIFO, so a
//!   short buffer desynchronises nothing.
```

`aid == 0` would compute `(0 - 1) * 1024` — in C, a wild pointer; in Rust,
either a panic or, with wrapping, a memory-safety incident waiting to happen.
Rejecting it is not paranoia, it is the port doing its job.

The semaphore itself had to be written from scratch, since Rust's standard
library has no counting semaphore
([local/semaphore.rs:1-11](core/src/hw/net/local/semaphore.rs#L1-L11)).

---

## 18.13 Diagnostics

Because MP failures are silent — a room simply never appears — the module
carries more instrumentation than any other.

```text
   LUNARIS_WIFI_DEBUG=1   TX/RX/beacon path trace, independent of the log
                          crate's level (the frontends default to Off)
   LUNARIS_MP_DIAG        periodic handshake-counter dump
   LUNARIS_SPI_TRACE      IR/SPI selector bytes (Chapter 15)
```

```rust
pub(super) fn debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("LUNARIS_WIFI_DEBUG").is_some())
}
```

([wifi/mod.rs:66-74](core/src/hw/net/wifi/mod.rs#L66-L74)) — cached in a
`OnceLock`, because `tick` runs 125,000 times per emulated second and cannot
re-read the environment each time.

`MpDiag` is a structured snapshot the _frontend_ can render, not just a log line
([wifi/mod.rs:976-993](core/src/hw/net/wifi/mod.rs#L976-L993)):

```rust
    pub fn diag_snapshot(&self) -> diag::MpDiag {
        let mut snapshot = self.diag;
        snapshot.channel = self.cur_channel;
        snapshot.is_mp = self.is_mp;
        snapshot.is_mp_client = self.is_mp_client;
        snapshot.aid = self.ioport(W_AIDLow);
        snapshot.transport_installed = self.transport.is_some();
```

```text
   how far did the handshake get?
   ──────────────────────────────
   transport_installed = false  →  the frontend never wired up a transport
   channel = 0                  →  the radio never picked a channel
   is_mp = false                →  the game never entered MP mode
   aid = 0                      →  associated? no. Beacon/auth/assoc failed
   aid = 1..15                  →  associated; problems are past this point
```

---

## 18.14 Where this stands

Implemented and verified:

- Wi-Fi register file, power sequencing, the 8 µs tick with drift correction
- TX slots and the RX ring
- Beacon / auth / assoc, AID assignment
- CMD / reply / ACK exchange
- `LocalMp` in-process backend, `LoopbackTransport` for tests
- LAN rooms and transport in [`gui/net`](gui/net/)
- Clock-epoch synchronisation for consoles started at different times

Not implemented:

- **Internet play (Nintendo WFC).** The `NetDriver` trait is the seam; only
  `NullNetDriver` and `LoopbackNetDriver` exist. melonDS's PCap and Slirp
  drivers were deliberately not ported (§18.1).
- **WEP / WPA association with a real access point.**
- **DSi-specific Wi-Fi.**
- **`Occasion::WirelessInterrupt` DMA** (Chapter 8) — the Wi-Fi path does not
  use DMA.
- **Real-hardware RF timing.** Channel switching, carrier sense and retry
  behaviour are approximated.

Verification status is honest: local MP has been exercised with the in-process
loopback and a purpose-built tiny ROM, and the frame path matches melonDS
round-for-round, but **end-to-end play of a retail multiplayer game has not
been confirmed**. Design notes and the current investigation live in
`docs/design/local_mp/` and `docs/design/ds-wifi-primer.md`.

---

[← 17. RTC, Keypad and the Maths Units](17_rtc_keypad_math.md) | [Next: 19. Savestates →](19_savestates.md)
