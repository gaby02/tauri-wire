//! Thread-safe stream buffer for accumulate-and-drain patterns.
//!
//! Producer threads push wire messages from any thread.
//! The consumer (Tauri command handler, render loop, or WebSocket sender)
//! calls [`drain()`](StreamBuffer::drain) to get a single buffer of
//! concatenated frames — one IPC call per render tick.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::protocol::{WireMessage, encode_frame_into};

/// Thread-safe accumulator for binary wire frames.
///
/// # Usage patterns
///
/// **Poll (Tauri command):**
/// ```rust,ignore
/// #[tauri::command]
/// fn poll(stream: tauri::State<StreamBuffer>) -> tauri::ipc::Response {
///     tauri::ipc::Response::new(stream.drain())
/// }
/// ```
///
/// **Timer-driven push:**
/// ```rust,ignore
/// loop {
///     std::thread::sleep(Duration::from_millis(16)); // ~60fps
///     let bytes = stream.drain();
///     if !bytes.is_empty() {
///         channel.send(bytes).ok();
///     }
/// }
/// ```
pub struct StreamBuffer {
    buf: Mutex<Vec<u8>>,
    /// Per-msg_type sequence counters. Indexed by msg_type (0-255).
    /// We use a fixed array — 256 × 4 bytes = 1KB, cheaper than a HashMap.
    sequences: [AtomicU32; 256],
    max_bytes: Option<usize>,
}

/// Error returned by [`StreamBuffer::try_push`] and [`StreamBuffer::try_push_one`]
/// when the buffer would exceed the configured maximum size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferFull;

impl std::fmt::Display for BufferFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stream buffer full")
    }
}

impl std::error::Error for BufferFull {}

impl StreamBuffer {
    /// Create a new stream buffer with default 64KB pre-allocation.
    pub fn new() -> Self {
        Self::with_capacity(65_536)
    }

    /// Create a new stream buffer with a specific byte capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: Mutex::new(Vec::with_capacity(capacity)),
            sequences: std::array::from_fn(|_| AtomicU32::new(0)),
            max_bytes: None,
        }
    }

    /// Create a bounded stream buffer. Pushes beyond `max_bytes` will fail
    /// (via `try_push` / `try_push_one`) or be silently accepted (via `push` / `push_one`).
    pub fn bounded(capacity: usize, max_bytes: usize) -> Self {
        Self {
            buf: Mutex::new(Vec::with_capacity(capacity)),
            sequences: std::array::from_fn(|_| AtomicU32::new(0)),
            max_bytes: Some(max_bytes),
        }
    }

    fn lock_buf(&self) -> MutexGuard<'_, Vec<u8>> {
        self.buf.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Push one or more messages of the same type into the buffer.
    ///
    /// Each call creates one frame. The sequence number auto-increments
    /// per `M::MSG_TYPE`.
    pub fn push<M: WireMessage>(&self, items: &[M]) {
        if items.is_empty() {
            return;
        }
        let seq = self.sequences[M::MSG_TYPE as usize].fetch_add(1, Ordering::Relaxed);
        let mut buf = self.lock_buf();
        encode_frame_into(&mut buf, seq, items);
    }

    /// Push a single message. Convenience for one-at-a-time streams.
    pub fn push_one<M: WireMessage>(&self, item: &M) {
        let seq = self.sequences[M::MSG_TYPE as usize].fetch_add(1, Ordering::Relaxed);
        let mut buf = self.lock_buf();
        encode_frame_into(&mut buf, seq, std::slice::from_ref(item));
    }

    /// Push a pre-encoded frame (raw bytes including header).
    ///
    /// Use this when you already have a frame from [`encode_frame`](crate::encode_frame)
    /// or received from another source.
    pub fn push_raw(&self, frame_bytes: &[u8]) {
        let mut buf = self.lock_buf();
        buf.extend_from_slice(frame_bytes);
    }

    /// Drain the buffer, returning all accumulated frame bytes.
    ///
    /// Returns an empty `Vec` if nothing was pushed since the last drain.
    /// The caller receives the old allocation; the internal buffer gets a
    /// fresh allocation of the same capacity.
    #[must_use]
    pub fn drain(&self) -> Vec<u8> {
        let mut buf = self.lock_buf();
        let mut out = Vec::with_capacity(buf.capacity());
        std::mem::swap(&mut *buf, &mut out);
        out
    }

    /// Number of bytes currently buffered.
    pub fn buffered_bytes(&self) -> usize {
        self.lock_buf().len()
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.lock_buf().is_empty()
    }

    /// Try to push one or more messages, respecting the max_bytes bound.
    ///
    /// Returns `Err(BufferFull)` if the buffer would exceed the limit.
    /// The sequence counter is **not** incremented on failure.
    pub fn try_push<M: WireMessage>(&self, items: &[M]) -> Result<(), BufferFull> {
        if items.is_empty() {
            return Ok(());
        }
        let payload_len: usize = items.iter().map(|m| m.encoded_size()).sum();
        let frame_len = crate::FRAME_HEADER_SIZE + payload_len;
        let mut buf = self.lock_buf();
        if let Some(max) = self.max_bytes {
            if buf.len() + frame_len > max {
                return Err(BufferFull);
            }
        }
        let seq = self.sequences[M::MSG_TYPE as usize].fetch_add(1, Ordering::Relaxed);
        encode_frame_into(&mut buf, seq, items);
        Ok(())
    }

    /// Try to push a single message, respecting the max_bytes bound.
    ///
    /// Returns `Err(BufferFull)` if the buffer would exceed the limit.
    pub fn try_push_one<M: WireMessage>(&self, item: &M) -> Result<(), BufferFull> {
        self.try_push(std::slice::from_ref(item))
    }

    /// Current sequence number for a given message type.
    pub fn sequence_of(&self, msg_type: u8) -> u32 {
        self.sequences[msg_type as usize].load(Ordering::Relaxed)
    }
}

impl Default for StreamBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ping(u64);
    impl WireMessage for Ping {
        const MSG_TYPE: u8 = 1;
        fn encode(&self, buf: &mut Vec<u8>) { buf.extend_from_slice(&self.0.to_le_bytes()); }
        fn encoded_size(&self) -> usize { 8 }
        fn decode(data: &[u8]) -> Option<(Self, usize)> {
            let v = u64::from_le_bytes(data.get(..8)?.try_into().ok()?);
            Some((Self(v), 8))
        }
    }

    struct Pong(u32);
    impl WireMessage for Pong {
        const MSG_TYPE: u8 = 2;
        fn encode(&self, buf: &mut Vec<u8>) { buf.extend_from_slice(&self.0.to_le_bytes()); }
        fn encoded_size(&self) -> usize { 4 }
        fn decode(data: &[u8]) -> Option<(Self, usize)> {
            let v = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
            Some((Self(v), 4))
        }
    }

    #[test]
    fn push_and_drain() {
        let stream = StreamBuffer::new();

        stream.push_one(&Ping(100));
        stream.push_one(&Pong(200));

        assert!(!stream.is_empty());
        let bytes = stream.drain();
        assert!(stream.is_empty());

        // Two frames: (9 + 8) + (9 + 4) = 30 bytes
        assert_eq!(bytes.len(), 30);
    }

    #[test]
    fn drain_returns_empty_when_nothing_pushed() {
        let stream = StreamBuffer::new();
        assert!(stream.drain().is_empty());
    }

    #[test]
    fn sequences_increment_per_type() {
        let stream = StreamBuffer::new();

        stream.push_one(&Ping(1));
        stream.push_one(&Ping(2));
        stream.push_one(&Pong(3));

        assert_eq!(stream.sequence_of(Ping::MSG_TYPE), 2);
        assert_eq!(stream.sequence_of(Pong::MSG_TYPE), 1);
    }

    #[test]
    fn multi_item_frame() {
        let stream = StreamBuffer::new();
        stream.push(&[Ping(10), Ping(20), Ping(30)]);

        let bytes = stream.drain();
        // One frame: 9 header + 3×8 payload = 33 bytes
        assert_eq!(bytes.len(), 33);
    }

    #[test]
    fn concurrent_push() {
        use std::sync::Arc;
        use std::thread;

        let stream = Arc::new(StreamBuffer::new());
        let mut handles = vec![];

        for t in 0..8 {
            let s = Arc::clone(&stream);
            handles.push(thread::spawn(move || {
                for i in 0..100 {
                    s.push_one(&Ping((t * 1000 + i) as u64));
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let bytes = stream.drain();
        // 800 frames, each 9 + 8 = 17 bytes
        assert_eq!(bytes.len(), 800 * 17);
    }

    #[test]
    fn bounded_rejects_when_full() {
        // 9-byte header + 8-byte payload = 17 bytes per frame
        let stream = StreamBuffer::bounded(1024, 20);
        assert!(stream.try_push_one(&Ping(1)).is_ok());
        assert_eq!(stream.try_push_one(&Ping(2)), Err(super::BufferFull));
        // Drain frees space
        let _ = stream.drain();
        assert!(stream.try_push_one(&Ping(3)).is_ok());
    }

    #[test]
    fn unbounded_push_always_succeeds() {
        let stream = StreamBuffer::new();
        // try_push on unbounded buffer always succeeds
        assert!(stream.try_push_one(&Ping(1)).is_ok());
        assert!(stream.try_push_one(&Ping(2)).is_ok());
    }

    #[test]
    fn push_raw_roundtrip() {
        let stream = StreamBuffer::new();

        // Encode a frame externally, then push raw bytes
        let frame = crate::encode_frame(&[Ping(999)]);
        stream.push_raw(&frame);

        let bytes = stream.drain();
        assert_eq!(bytes.len(), frame.len());

        let mut reader = crate::FrameReader::new(&bytes);
        let view = reader.next_frame().unwrap();
        let items = view.decode_all::<Ping>().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, 999);
    }
}
