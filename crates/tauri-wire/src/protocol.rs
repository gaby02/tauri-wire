//! Wire protocol — frame format, encoding, and the [`WireMessage`] trait.
//!
//! ## Frame layout
//!
//! ```text
//! [u8  msg_type]      // user-defined tag (0-255)
//! [u32 sequence]      // monotonic per msg_type, little-endian
//! [u32 payload_len]   // bytes of payload after this header, little-endian
//! [payload ...]       // exactly payload_len bytes
//! ```
//!
//! Frames are self-contained. A buffer of concatenated frames is a valid stream.
//! Unknown msg_type values can be skipped by jumping payload_len bytes.

/// Frame header size in bytes.
pub const FRAME_HEADER_SIZE: usize = 9;

/// Trait for user-defined binary message types.
///
/// Implement this for each data type you want to stream.
/// All encoding must be **little-endian** for `DataView` compatibility on the JS side.
///
/// # Fixed-size vs variable-size
///
/// - **Fixed-size:** `encoded_size()` returns a constant. Decoders can compute
///   item count as `payload_len / size`.
/// - **Variable-size:** Items must be self-delimiting within the payload
///   (e.g., length-prefixed sub-arrays). `encoded_size()` varies per instance.
pub trait WireMessage: Sized {
    /// Unique message type tag (0-255). Must not collide with other types
    /// in the same stream.
    const MSG_TYPE: u8;

    /// Encode this message into the buffer.
    /// Must append exactly `encoded_size()` bytes.
    fn encode(&self, buf: &mut Vec<u8>);

    /// Number of bytes this instance encodes to.
    fn encoded_size(&self) -> usize;

    /// Decode one message from the front of `data`.
    /// Returns `(decoded_message, bytes_consumed)`.
    /// Returns `None` if the buffer is too short or data is invalid.
    fn decode(data: &[u8]) -> Option<(Self, usize)>;
}

/// Parsed frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub msg_type: u8,
    pub sequence: u32,
    pub payload_len: u32,
}

impl FrameHeader {
    /// Decode a frame header from a byte slice.
    #[inline]
    pub fn read(data: &[u8]) -> Option<Self> {
        if data.len() < FRAME_HEADER_SIZE {
            return None;
        }
        Some(Self {
            msg_type: data[0],
            sequence: u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
            payload_len: u32::from_le_bytes([data[5], data[6], data[7], data[8]]),
        })
    }

    /// Write this header into the buffer (9 bytes).
    #[inline]
    pub fn write(&self, buf: &mut Vec<u8>) {
        buf.push(self.msg_type);
        buf.extend_from_slice(&self.sequence.to_le_bytes());
        buf.extend_from_slice(&self.payload_len.to_le_bytes());
    }

    /// Total frame size (header + payload).
    #[inline]
    pub fn frame_size(&self) -> usize {
        FRAME_HEADER_SIZE + self.payload_len as usize
    }
}

/// Encode a slice of messages into a single frame.
///
/// Returns the raw frame bytes (header + payload). This is the unit you send
/// through `tauri::ipc::Channel`, a WebSocket, or accumulate in a [`StreamBuffer`].
///
/// Note: the sequence number is set to 0. Use [`StreamBuffer`] for auto-incrementing
/// sequences, or [`encode_frame_into`] to set it manually.
///
/// [`StreamBuffer`]: crate::StreamBuffer
#[must_use]
pub fn encode_frame<M: WireMessage>(items: &[M]) -> Vec<u8> {
    let payload_len: usize = items.iter().map(|m| m.encoded_size()).sum();
    let mut buf = Vec::with_capacity(FRAME_HEADER_SIZE + payload_len);
    encode_frame_into(&mut buf, 0, items);
    buf
}

/// Encode a slice of messages into a single frame with a specific sequence number.
///
/// Like [`encode_frame`] but lets you set the sequence number.
#[must_use]
pub fn encode_frame_with_seq<M: WireMessage>(sequence: u32, items: &[M]) -> Vec<u8> {
    let payload_len: usize = items.iter().map(|m| m.encoded_size()).sum();
    let mut buf = Vec::with_capacity(FRAME_HEADER_SIZE + payload_len);
    encode_frame_into(&mut buf, sequence, items);
    buf
}

/// Encode a frame with a specific sequence number, appending to an existing buffer.
///
/// # Panics
///
/// Panics if the total payload exceeds `u32::MAX` bytes (~4 GB).
pub fn encode_frame_into<M: WireMessage>(buf: &mut Vec<u8>, sequence: u32, items: &[M]) {
    let payload_len: usize = items.iter().map(|m| m.encoded_size()).sum();
    assert!(
        payload_len <= u32::MAX as usize,
        "tauri-wire: payload too large ({payload_len} bytes, max {})",
        u32::MAX
    );
    let header = FrameHeader {
        msg_type: M::MSG_TYPE,
        sequence,
        payload_len: payload_len as u32,
    };
    header.write(buf);
    for item in items {
        item.encode(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Val(u64);

    impl WireMessage for Val {
        const MSG_TYPE: u8 = 42;
        fn encode(&self, buf: &mut Vec<u8>) {
            buf.extend_from_slice(&self.0.to_le_bytes());
        }
        fn encoded_size(&self) -> usize { 8 }
        fn decode(data: &[u8]) -> Option<(Self, usize)> {
            if data.len() < 8 { return None; }
            let v = u64::from_le_bytes(data[..8].try_into().ok()?);
            Some((Self(v), 8))
        }
    }

    #[test]
    fn header_roundtrip() {
        let h = FrameHeader { msg_type: 7, sequence: 999, payload_len: 4096 };
        let mut buf = Vec::new();
        h.write(&mut buf);
        assert_eq!(buf.len(), FRAME_HEADER_SIZE);
        assert_eq!(FrameHeader::read(&buf), Some(h));
    }

    #[test]
    fn encode_frame_single() {
        let frame = encode_frame(&[Val(0xDEAD)]);
        assert_eq!(frame.len(), FRAME_HEADER_SIZE + 8);

        let h = FrameHeader::read(&frame).unwrap();
        assert_eq!(h.msg_type, 42);
        assert_eq!(h.payload_len, 8);

        let (v, _) = Val::decode(&frame[FRAME_HEADER_SIZE..]).unwrap();
        assert_eq!(v.0, 0xDEAD);
    }

    #[test]
    fn encode_frame_batch() {
        let frame = encode_frame(&[Val(1), Val(2), Val(3)]);
        let h = FrameHeader::read(&frame).unwrap();
        assert_eq!(h.payload_len, 24);

        let payload = &frame[FRAME_HEADER_SIZE..];
        let (v1, n) = Val::decode(payload).unwrap();
        let (v2, m) = Val::decode(&payload[n..]).unwrap();
        let (v3, _) = Val::decode(&payload[n + m..]).unwrap();
        assert_eq!((v1.0, v2.0, v3.0), (1, 2, 3));
    }

    #[test]
    fn encode_frame_into_appends() {
        let mut buf = vec![0xFF]; // pre-existing data
        encode_frame_into(&mut buf, 5, &[Val(100)]);
        assert_eq!(buf[0], 0xFF); // untouched
        let h = FrameHeader::read(&buf[1..]).unwrap();
        assert_eq!(h.sequence, 5);
    }

    #[test]
    fn encode_frame_with_seq_sets_sequence() {
        let frame = encode_frame_with_seq(42, &[Val(1)]);
        let h = FrameHeader::read(&frame).unwrap();
        assert_eq!(h.sequence, 42);
        assert_eq!(h.msg_type, 42);
        assert_eq!(h.payload_len, 8);
    }
}
