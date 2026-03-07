//! Example: Using tauri-wire for financial market data.
//!
//! Shows how to define domain-specific wire types (ticks, bars, order book)
//! and stream them through a StreamBuffer.

use tauri_wire::{FrameReader, StreamBuffer, WireMessage};

// ── Domain types ───────────────────────────────────────────────────

/// Trade tick: 32 bytes (padded for alignment).
struct Tick {
    ts_ms: i64,
    price: f64,
    size: f64,
    side: u8, // 0=unknown, 1=buy, 2=sell
}

impl WireMessage for Tick {
    const MSG_TYPE: u8 = 1;

    fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.ts_ms.to_le_bytes());
        buf.extend_from_slice(&self.price.to_le_bytes());
        buf.extend_from_slice(&self.size.to_le_bytes());
        buf.push(self.side);
        buf.extend_from_slice(&[0u8; 7]); // pad to 32
    }

    fn encoded_size(&self) -> usize { 32 }

    fn decode(data: &[u8]) -> Option<(Self, usize)> {
        if data.len() < 32 { return None; }
        Some((Self {
            ts_ms: i64::from_le_bytes(data[0..8].try_into().ok()?),
            price: f64::from_le_bytes(data[8..16].try_into().ok()?),
            size: f64::from_le_bytes(data[16..24].try_into().ok()?),
            side: data[24],
        }, 32))
    }
}

/// OHLCV bar: 48 bytes.
struct Bar {
    ts_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

impl WireMessage for Bar {
    const MSG_TYPE: u8 = 2;

    fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.ts_ms.to_le_bytes());
        buf.extend_from_slice(&self.open.to_le_bytes());
        buf.extend_from_slice(&self.high.to_le_bytes());
        buf.extend_from_slice(&self.low.to_le_bytes());
        buf.extend_from_slice(&self.close.to_le_bytes());
        buf.extend_from_slice(&self.volume.to_le_bytes());
    }

    fn encoded_size(&self) -> usize { 48 }

    fn decode(data: &[u8]) -> Option<(Self, usize)> {
        if data.len() < 48 { return None; }
        Some((Self {
            ts_ms: i64::from_le_bytes(data[0..8].try_into().ok()?),
            open: f64::from_le_bytes(data[8..16].try_into().ok()?),
            high: f64::from_le_bytes(data[16..24].try_into().ok()?),
            low: f64::from_le_bytes(data[24..32].try_into().ok()?),
            close: f64::from_le_bytes(data[32..40].try_into().ok()?),
            volume: f64::from_le_bytes(data[40..48].try_into().ok()?),
        }, 48))
    }
}

/// Order book snapshot — variable-size message.
///
/// Wire layout: [i64 ts][u16 symbol_id][u16 bid_count][u16 ask_count][levels...]
/// Each level: [f64 price][f64 size] = 16 bytes.
struct BookSnapshot {
    ts_ms: i64,
    symbol_id: u16,
    bids: Vec<(f64, f64)>, // (price, size)
    asks: Vec<(f64, f64)>,
}

impl WireMessage for BookSnapshot {
    const MSG_TYPE: u8 = 3;

    fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.ts_ms.to_le_bytes());
        buf.extend_from_slice(&self.symbol_id.to_le_bytes());
        buf.extend_from_slice(&(self.bids.len() as u16).to_le_bytes());
        buf.extend_from_slice(&(self.asks.len() as u16).to_le_bytes());
        for &(price, size) in &self.bids {
            buf.extend_from_slice(&price.to_le_bytes());
            buf.extend_from_slice(&size.to_le_bytes());
        }
        for &(price, size) in &self.asks {
            buf.extend_from_slice(&price.to_le_bytes());
            buf.extend_from_slice(&size.to_le_bytes());
        }
    }

    fn encoded_size(&self) -> usize {
        8 + 2 + 2 + 2 + (self.bids.len() + self.asks.len()) * 16
    }

    fn decode(data: &[u8]) -> Option<(Self, usize)> {
        if data.len() < 14 { return None; }
        let ts_ms = i64::from_le_bytes(data[0..8].try_into().ok()?);
        let symbol_id = u16::from_le_bytes(data[8..10].try_into().ok()?);
        let bid_count = u16::from_le_bytes(data[10..12].try_into().ok()?) as usize;
        let ask_count = u16::from_le_bytes(data[12..14].try_into().ok()?) as usize;

        let total = 14 + (bid_count + ask_count) * 16;
        if data.len() < total { return None; }

        let mut offset = 14;
        let mut bids = Vec::with_capacity(bid_count);
        for _ in 0..bid_count {
            let price = f64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            let size = f64::from_le_bytes(data[offset + 8..offset + 16].try_into().ok()?);
            bids.push((price, size));
            offset += 16;
        }
        let mut asks = Vec::with_capacity(ask_count);
        for _ in 0..ask_count {
            let price = f64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            let size = f64::from_le_bytes(data[offset + 8..offset + 16].try_into().ok()?);
            asks.push((price, size));
            offset += 16;
        }

        Some((Self { ts_ms, symbol_id, bids, asks }, total))
    }
}

// ── Demo ───────────────────────────────────────────────────────────

fn main() {
    let stream = StreamBuffer::new();

    // Simulate market feed pushing data
    stream.push(&[
        Tick { ts_ms: 1_700_000_000_000, price: 42_567.50, size: 0.1, side: 1 },
        Tick { ts_ms: 1_700_000_000_001, price: 42_567.75, size: 0.3, side: 2 },
    ]);

    stream.push(&[Bar {
        ts_ms: 1_700_000_000_000,
        open: 42_500.0,
        high: 42_600.0,
        low: 42_450.0,
        close: 42_567.50,
        volume: 1_234.5,
    }]);

    stream.push_one(&BookSnapshot {
        ts_ms: 1_700_000_000_002,
        symbol_id: 1,
        bids: vec![(42_567.0, 5.0), (42_566.0, 3.0)],
        asks: vec![(42_568.0, 4.0), (42_569.0, 2.0)],
    });

    // Drain and decode (simulates Tauri command handler)
    let bytes = stream.drain();
    println!("Wire bytes: {} (3 frames)", bytes.len());

    let mut reader = FrameReader::new(&bytes);
    while let Some(frame) = reader.next_frame() {
        match frame.header.msg_type {
            Tick::MSG_TYPE => {
                let ticks = frame.decode_all::<Tick>().unwrap();
                for t in &ticks {
                    let side = match t.side { 1 => "BUY", 2 => "SELL", _ => "?" };
                    println!("  Tick: {} @ {} ({})", t.size, t.price, side);
                }
            }
            Bar::MSG_TYPE => {
                let bars = frame.decode_all::<Bar>().unwrap();
                for b in &bars {
                    println!("  Bar: O={} H={} L={} C={} V={}", b.open, b.high, b.low, b.close, b.volume);
                }
            }
            BookSnapshot::MSG_TYPE => {
                let books = frame.decode_all::<BookSnapshot>().unwrap();
                for book in &books {
                    println!("  Book (sym {}): {} bids, {} asks", book.symbol_id, book.bids.len(), book.asks.len());
                }
            }
            other => println!("  Unknown type: {}", other),
        }
    }
}
