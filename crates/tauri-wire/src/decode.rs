//! Zero-copy frame reader for decoding streams.
//!
//! [`FrameReader`] iterates over frames in a byte buffer without copying.
//! Each [`FrameView`] gives you the header and a slice of the payload.

use crate::protocol::{FrameHeader, FRAME_HEADER_SIZE};

/// Zero-copy view of a single frame within a buffer.
#[derive(Debug)]
pub struct FrameView<'a> {
    /// Parsed frame header.
    pub header: FrameHeader,
    /// Raw payload bytes (not including the header).
    pub payload: &'a [u8],
}

impl<'a> FrameView<'a> {
    /// Decode all items of type `M` from this frame's payload.
    ///
    /// Returns `None` if any item fails to decode.
    pub fn decode_all<M: crate::WireMessage>(&self) -> Option<Vec<M>> {
        let mut items = Vec::new();
        let mut offset = 0;
        while offset < self.payload.len() {
            let (item, consumed) = M::decode(&self.payload[offset..])?;
            if consumed == 0 {
                return None; // prevent infinite loop from buggy WireMessage impl
            }
            items.push(item);
            offset += consumed;
        }
        Some(items)
    }

    /// Number of fixed-size items in the payload (payload_len / item_size).
    ///
    /// Only meaningful for fixed-size message types.
    pub fn item_count(&self, item_size: usize) -> usize {
        if item_size == 0 {
            return 0;
        }
        self.payload.len() / item_size
    }
}

/// Iterator over frames in a byte buffer.
///
/// Yields [`FrameView`] references without copying the payload.
///
/// ```rust
/// use tauri_wire::FrameReader;
///
/// fn process(bytes: &[u8]) {
///     let mut reader = FrameReader::new(bytes);
///     while let Some(frame) = reader.next_frame() {
///         match frame.header.msg_type {
///             1 => { /* decode type 1 items from frame.payload */ }
///             2 => { /* decode type 2 items from frame.payload */ }
///             _ => { /* skip unknown types */ }
///         }
///     }
/// }
/// ```
pub struct FrameReader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> FrameReader<'a> {
    /// Create a reader over a byte buffer containing concatenated frames.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    /// Read the next frame, advancing the internal cursor.
    ///
    /// Returns `None` when the buffer is exhausted or contains
    /// an incomplete frame.
    pub fn next_frame(&mut self) -> Option<FrameView<'a>> {
        let remaining = &self.data[self.offset..];
        let header = FrameHeader::read(remaining)?;
        let payload_end = FRAME_HEADER_SIZE + header.payload_len as usize;

        if remaining.len() < payload_end {
            return None; // incomplete frame
        }

        let payload = &remaining[FRAME_HEADER_SIZE..payload_end];
        self.offset += payload_end;

        Some(FrameView { header, payload })
    }

    /// Number of bytes remaining in the buffer.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    /// Whether the reader has consumed all bytes.
    pub fn is_exhausted(&self) -> bool {
        self.remaining() == 0
    }
}

impl<'a> Iterator for FrameReader<'a> {
    type Item = FrameView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_frame()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{WireMessage, encode_frame_into};

    struct Num(u32);
    impl WireMessage for Num {
        const MSG_TYPE: u8 = 10;
        fn encode(&self, buf: &mut Vec<u8>) { buf.extend_from_slice(&self.0.to_le_bytes()); }
        fn encoded_size(&self) -> usize { 4 }
        fn decode(data: &[u8]) -> Option<(Self, usize)> {
            let v = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
            Some((Self(v), 4))
        }
    }

    #[test]
    fn read_single_frame() {
        let mut buf = Vec::new();
        encode_frame_into(&mut buf, 0, &[Num(42)]);

        let mut reader = FrameReader::new(&buf);
        let frame = reader.next_frame().unwrap();
        assert_eq!(frame.header.msg_type, 10);
        assert_eq!(frame.payload.len(), 4);

        let items = frame.decode_all::<Num>().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, 42);

        assert!(reader.next_frame().is_none());
        assert!(reader.is_exhausted());
    }

    #[test]
    fn read_multiple_frames() {
        let mut buf = Vec::new();
        encode_frame_into(&mut buf, 0, &[Num(1), Num(2)]);
        encode_frame_into(&mut buf, 1, &[Num(3)]);

        let mut reader = FrameReader::new(&buf);

        let f1 = reader.next_frame().unwrap();
        assert_eq!(f1.header.sequence, 0);
        assert_eq!(f1.item_count(4), 2);

        let f2 = reader.next_frame().unwrap();
        assert_eq!(f2.header.sequence, 1);
        let items = f2.decode_all::<Num>().unwrap();
        assert_eq!(items[0].0, 3);

        assert!(reader.next_frame().is_none());
    }

    #[test]
    fn skip_unknown_frame_types() {
        // Type 10 and type 99 (unknown)
        struct Unknown;
        impl WireMessage for Unknown {
            const MSG_TYPE: u8 = 99;
            fn encode(&self, buf: &mut Vec<u8>) { buf.extend_from_slice(&[0u8; 16]); }
            fn encoded_size(&self) -> usize { 16 }
            fn decode(_: &[u8]) -> Option<(Self, usize)> { Some((Self, 16)) }
        }

        let mut buf = Vec::new();
        encode_frame_into(&mut buf, 0, &[Num(1)]);
        encode_frame_into(&mut buf, 0, &[Unknown]);
        encode_frame_into(&mut buf, 1, &[Num(2)]);

        let mut reader = FrameReader::new(&buf);
        let mut nums = vec![];

        while let Some(frame) = reader.next_frame() {
            if frame.header.msg_type == Num::MSG_TYPE {
                for item in frame.decode_all::<Num>().unwrap() {
                    nums.push(item.0);
                }
            }
            // unknown types are silently skipped
        }

        assert_eq!(nums, vec![1, 2]);
    }

    #[test]
    fn handles_truncated_buffer() {
        let mut buf = Vec::new();
        encode_frame_into(&mut buf, 0, &[Num(42)]);
        buf.truncate(buf.len() - 2); // corrupt: payload cut short

        let mut reader = FrameReader::new(&buf);
        assert!(reader.next_frame().is_none()); // safely returns None
    }

    #[test]
    fn decode_all_guards_zero_consumed() {
        // A buggy WireMessage that returns consumed=0 should not infinite-loop
        struct Bad;
        impl WireMessage for Bad {
            const MSG_TYPE: u8 = 77;
            fn encode(&self, buf: &mut Vec<u8>) { buf.push(0); }
            fn encoded_size(&self) -> usize { 1 }
            fn decode(_data: &[u8]) -> Option<(Self, usize)> {
                Some((Self, 0)) // bug: consumed 0 bytes
            }
        }

        let mut buf = Vec::new();
        encode_frame_into(&mut buf, 0, &[Bad]);

        let mut reader = FrameReader::new(&buf);
        let frame = reader.next_frame().unwrap();
        // Should return None (abort) instead of looping forever
        assert!(frame.decode_all::<Bad>().is_none());
    }

    #[test]
    fn iterator_trait_works() {
        let mut buf = Vec::new();
        encode_frame_into(&mut buf, 0, &[Num(1)]);
        encode_frame_into(&mut buf, 1, &[Num(2)]);
        encode_frame_into(&mut buf, 2, &[Num(3)]);

        let reader = FrameReader::new(&buf);
        let seqs: Vec<u32> = reader.map(|f| f.header.sequence).collect();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    #[test]
    fn empty_frame_is_valid() {
        // A frame with zero items has payload_len=0 and is valid
        let frame = crate::encode_frame::<Num>(&[]);
        assert_eq!(frame.len(), crate::FRAME_HEADER_SIZE);

        let mut reader = FrameReader::new(&frame);
        let view = reader.next_frame().unwrap();
        assert_eq!(view.header.payload_len, 0);
        assert_eq!(view.payload.len(), 0);

        let items = view.decode_all::<Num>().unwrap();
        assert!(items.is_empty());
    }
}
