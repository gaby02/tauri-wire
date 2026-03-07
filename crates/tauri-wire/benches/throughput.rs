//! Benchmark: JSON vs tauri-wire binary encoding/decoding throughput.

use criterion::{criterion_group, criterion_main, Criterion};
use serde::{Deserialize, Serialize};
use std::hint::black_box;
use tauri_wire::{FrameReader, StreamBuffer, WireMessage, encode_frame};

// ── Wire message ───────────────────────────────────────────────────

struct Sample {
    ts: i64,
    a: f64,
    b: f64,
    c: u32,
    d: u32,
}

impl WireMessage for Sample {
    const MSG_TYPE: u8 = 1;

    fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.ts.to_le_bytes());
        buf.extend_from_slice(&self.a.to_le_bytes());
        buf.extend_from_slice(&self.b.to_le_bytes());
        buf.extend_from_slice(&self.c.to_le_bytes());
        buf.extend_from_slice(&self.d.to_le_bytes());
    }

    fn encoded_size(&self) -> usize { 32 }

    fn decode(data: &[u8]) -> Option<(Self, usize)> {
        if data.len() < 32 { return None; }
        Some((Self {
            ts: i64::from_le_bytes(data[0..8].try_into().ok()?),
            a: f64::from_le_bytes(data[8..16].try_into().ok()?),
            b: f64::from_le_bytes(data[16..24].try_into().ok()?),
            c: u32::from_le_bytes(data[24..28].try_into().ok()?),
            d: u32::from_le_bytes(data[28..32].try_into().ok()?),
        }, 32))
    }
}

// ── JSON equivalent ────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct JsonSample {
    ts: i64,
    a: f64,
    b: f64,
    c: u32,
    d: u32,
}

// ── Helpers ────────────────────────────────────────────────────────

fn make_wire(n: usize) -> Vec<Sample> {
    (0..n)
        .map(|i| Sample {
            ts: 1_700_000_000_000 + i as i64,
            a: 100.0 + i as f64 * 0.01,
            b: 200.0 + i as f64 * 0.02,
            c: i as u32,
            d: (i * 7) as u32,
        })
        .collect()
}

fn make_json(n: usize) -> Vec<JsonSample> {
    (0..n)
        .map(|i| JsonSample {
            ts: 1_700_000_000_000 + i as i64,
            a: 100.0 + i as f64 * 0.01,
            b: 200.0 + i as f64 * 0.02,
            c: i as u32,
            d: (i * 7) as u32,
        })
        .collect()
}

// ── Benchmarks ─────────────────────────────────────────────────────

fn bench_encode_100(c: &mut Criterion) {
    let mut g = c.benchmark_group("encode_100");
    let wire = make_wire(100);
    let json = make_json(100);

    g.bench_function("binary", |b| {
        b.iter(|| black_box(encode_frame(black_box(&wire))));
    });
    g.bench_function("json", |b| {
        b.iter(|| black_box(serde_json::to_vec(black_box(&json)).unwrap()));
    });
    g.finish();
}

fn bench_decode_100(c: &mut Criterion) {
    let mut g = c.benchmark_group("decode_100");

    let wire = make_wire(100);
    let binary_buf = encode_frame(&wire);

    let json = make_json(100);
    let json_buf = serde_json::to_vec(&json).unwrap();

    g.bench_function("binary", |b| {
        b.iter(|| {
            let mut reader = FrameReader::new(black_box(&binary_buf));
            while let Some(frame) = reader.next_frame() {
                let items = frame.decode_all::<Sample>().unwrap();
                black_box(&items);
            }
        });
    });
    g.bench_function("json", |b| {
        b.iter(|| {
            let items: Vec<JsonSample> = serde_json::from_slice(black_box(&json_buf)).unwrap();
            black_box(&items);
        });
    });
    g.finish();
}

fn bench_stream_1000(c: &mut Criterion) {
    c.bench_function("stream_push_drain_1000", |b| {
        let stream = StreamBuffer::with_capacity(64_000);
        b.iter(|| {
            for i in 0..1000 {
                stream.push_one(&Sample {
                    ts: i, a: 1.0, b: 2.0, c: 3, d: 4,
                });
            }
            black_box(stream.drain());
        });
    });
}

fn bench_wire_sizes(c: &mut Criterion) {
    let mut g = c.benchmark_group("wire_size");

    let wire = make_wire(100);
    let json = make_json(100);

    let binary_size = encode_frame(&wire).len();
    let json_size = serde_json::to_vec(&json).unwrap().len();

    g.bench_function(&format!("binary_{binary_size}B"), |b| {
        b.iter(|| black_box(binary_size));
    });
    g.bench_function(&format!("json_{json_size}B"), |b| {
        b.iter(|| black_box(json_size));
    });
    g.finish();
}

criterion_group!(benches, bench_encode_100, bench_decode_100, bench_stream_1000, bench_wire_sizes);
criterion_main!(benches);
