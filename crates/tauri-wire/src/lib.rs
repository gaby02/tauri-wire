//! # tauri-wire
//!
//! Zero-parse binary wire protocol for Tauri IPC.
//!
//! Tauri's default IPC serializes everything as JSON. For high-frequency data
//! this becomes the bottleneck. `tauri-wire` provides a streaming binary protocol
//! where every frame is self-contained, independently sendable, and decodable
//! on the JS side via `DataView` — no `JSON.parse()`, no allocation.
//!
//! ## Design
//!
//! **Streaming-first.** Each frame is a standalone unit with a type tag, sequence
//! number, and length-prefixed payload. Frames can be:
//! - Pushed one at a time through `tauri::ipc::Channel` (push model)
//! - Accumulated in a [`StreamBuffer`] and drained per render frame (poll model)
//! - Sent over WebSocket, TCP, or written to a file
//!
//! **Generic.** You define your own message types by implementing [`WireMessage`].
//! The protocol doesn't know or care what's inside the payload.
//!
//! **Zero dependencies.** Uses only `std` — no serde, no proc-macros, no
//! allocator tricks.
//!
//! ## Wire format
//!
//! ```text
//! Frame (self-contained):
//!   [u8  msg_type]      ─── user-defined tag (0-255)
//!   [u32 sequence]      ─── monotonic per type, gaps = dropped frames
//!   [u32 payload_len]   ─── byte count after this header
//!   [payload ...]       ─── payload_len bytes of encoded items
//!
//! Frame header: 9 bytes. All integers little-endian.
//! ```
//!
//! Frames are concatenated in a buffer. The decoder iterates them by reading
//! header → skip `payload_len` bytes → next header. Unknown `msg_type` values
//! are safely skipped.
//!
//! ## Quick start
//!
//! ```rust
//! use tauri_wire::{WireMessage, StreamBuffer, FrameReader};
//!
//! // 1. Define a message type
//! struct Ping { ts: u64 }
//!
//! impl WireMessage for Ping {
//!     const MSG_TYPE: u8 = 1;
//!     fn encode(&self, buf: &mut Vec<u8>) {
//!         buf.extend_from_slice(&self.ts.to_le_bytes());
//!     }
//!     fn encoded_size(&self) -> usize { 8 }
//!     fn decode(data: &[u8]) -> Option<(Self, usize)> {
//!         let ts = u64::from_le_bytes(data.get(..8)?.try_into().ok()?);
//!         Some((Self { ts }, 8))
//!     }
//! }
//!
//! // 2. Stream: push from any thread, drain per frame
//! let stream = StreamBuffer::new();
//! stream.push(&[Ping { ts: 1 }, Ping { ts: 2 }]);
//!
//! let bytes = stream.drain();
//!
//! // 3. Decode (Rust side — JS side uses ts/decoder.ts)
//! let mut reader = FrameReader::new(&bytes);
//! while let Some(frame) = reader.next_frame() {
//!     assert_eq!(frame.header.msg_type, 1);
//!     // decode items from frame.payload
//! }
//! ```
//!
//! ## Tauri v2 integration
//!
//! ```rust,ignore
//! // Poll model — command returns raw bytes
//! #[tauri::command]
//! fn poll(stream: tauri::State<StreamBuffer>) -> tauri::ipc::Response {
//!     tauri::ipc::Response::new(stream.drain())
//! }
//!
//! // Push model — stream frames via Channel
//! #[tauri::command]
//! fn subscribe(channel: tauri::ipc::Channel<Vec<u8>>) {
//!     // encode single frame and send
//!     let frame = tauri_wire::encode_frame(&[Ping { ts: now() }]);
//!     channel.send(frame).ok();
//! }
//! ```

mod protocol;
mod stream;
mod decode;

pub use protocol::{WireMessage, FrameHeader, encode_frame, encode_frame_with_seq, encode_frame_into, FRAME_HEADER_SIZE};
pub use stream::{StreamBuffer, BufferFull};
pub use decode::{FrameReader, FrameView};

#[cfg(feature = "derive")]
pub use tauri_wire_derive::WireMessage;
