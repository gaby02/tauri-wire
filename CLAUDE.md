# tauri-wire — LLM Development Guide

> Binary framing protocol that replaces JSON serialization in Tauri IPC for high-frequency data streams.

This file helps AI coding assistants (Claude, Copilot, Cursor, etc.) understand and work with this crate effectively.

## What this crate is

`tauri-wire` is a **wire codec** — a framing + serialization layer for streaming binary data over Tauri's IPC. It is NOT an IPC transport itself. Tauri provides the pipe (`ipc::Response`, `Channel`); this crate provides the format of what goes through it.

Think of it as a lightweight alternative to Protocol Buffers or SBE, purpose-built for the "Rust backend → webview frontend" use case.

## Architecture

```
crates/
  tauri-wire/           Core library (zero dependencies)
    src/
      protocol.rs       WireMessage trait, FrameHeader, encode functions
      stream.rs         StreamBuffer (thread-safe accumulator with backpressure)
      decode.rs         FrameReader/FrameView (zero-copy frame iteration)
      lib.rs            Public re-exports
    ts/decoder.ts       TypeScript decoder (DataView-based, zero deps)
  tauri-wire-derive/    Proc-macro: #[derive(WireMessage)]
```

## Key types

| Type | Purpose |
|---|---|
| `WireMessage` | Trait — implement for each data type. Fields: `MSG_TYPE`, `encode()`, `decode()`, `encoded_size()` |
| `StreamBuffer` | Thread-safe frame accumulator. Push from any thread, `drain()` per render tick |
| `FrameReader` | Zero-copy iterator (`impl Iterator`) over concatenated frames |
| `encode_frame()` | One-shot encode a slice of messages into a frame `Vec<u8>` |
| `BufferFull` | Error from `try_push()` when bounded buffer is at capacity |

## Wire format

9-byte header, all little-endian:
- `u8 msg_type` — user-defined tag (0-255)
- `u32 sequence` — auto-incrementing per type
- `u32 payload_len` — bytes of payload after header

Frames are self-contained and concatenated in buffers. Unknown msg_types are safely skipped.

## Common tasks

### Add a new wire message type

With derive (recommended):
```rust
use tauri_wire::WireMessage;

#[derive(WireMessage)]
#[wire(msg_type = 5)]  // unique u8 tag
struct MyData {
    timestamp: i64,
    value: f64,
    flags: u32,
}
```

Supported field types: `u8`, `u16`, `u32`, `u64`, `i8`, `i16`, `i32`, `i64`, `f32`, `f64`, `bool`.

For variable-size or custom-layout types, implement `WireMessage` manually.

### Integrate with a Tauri v2 command

```rust
use tauri::State;
use tauri_wire::StreamBuffer;

// Register StreamBuffer as Tauri managed state
// In your app setup: app.manage(StreamBuffer::new());

// Poll endpoint — frontend calls this per animation frame
#[tauri::command]
fn poll(stream: State<StreamBuffer>) -> tauri::ipc::Response {
    tauri::ipc::Response::new(stream.drain())
}

// Producer — any thread pushes data
fn on_new_data(stream: &StreamBuffer, items: &[MyData]) {
    stream.push(items);
}
```

### Decode in TypeScript

```typescript
import { FrameIterator } from '@tauri-wire/decoder';

const buf: ArrayBuffer = await invoke('poll');
for (const frame of new FrameIterator(buf)) {
    if (frame.msgType === 5) {
        frame.forEachItem(20, (off) => {  // 20 = i64 + f64 + u32
            const ts    = frame.readI64(off);
            const value = frame.readF64(off + 8);
            const flags = frame.readU32(off + 16);
        });
    }
}
```

### Add backpressure

```rust
// Bounded buffer — rejects pushes beyond 1 MB
let stream = StreamBuffer::bounded(65_536, 1_000_000);

match stream.try_push(&items) {
    Ok(()) => {},
    Err(BufferFull) => { /* drop or queue elsewhere */ }
}
```

## Build & test

```sh
cargo test --workspace                    # core tests (20)
cargo test --workspace --features derive  # + derive tests (4)
cargo clippy --workspace --all-features   # lint
cargo bench -p tauri-wire                 # benchmarks
```

## Design constraints

- **Zero runtime dependencies.** Only `std`. The derive macro uses `syn`/`quote` at compile time only.
- **All encoding is little-endian.** This matches `DataView` defaults and x86/ARM native order.
- **Frames are self-contained.** No batch wrapper, no connection state, no handshake.
- **Wire format is unstable** until 1.0. The 9-byte header layout may change.
- **`msg_type` is a u8** — max 256 distinct message types per stream.
- **`payload_len` is a u32** — max ~4 GB per frame. Asserts on overflow.

## What NOT to do

- Don't use this for string-heavy or nested data — JSON is more ergonomic there.
- Don't add serde as a dependency — the whole point is zero-dep binary encoding.
- Don't change the frame header without updating both Rust and TypeScript sides.
- Don't use `push()` in tight loops without considering `try_push()` for backpressure.
