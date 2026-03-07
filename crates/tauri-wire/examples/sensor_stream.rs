//! Example: IoT sensor streaming.
//!
//! Shows that tauri-wire is domain-agnostic — works for any
//! high-frequency numeric data, not just finance.

use tauri_wire::{FrameReader, StreamBuffer, WireMessage};

struct SensorReading {
    ts_ms: i64,
    device_id: u16,
    temperature: f32,
    humidity: f32,
}

impl WireMessage for SensorReading {
    const MSG_TYPE: u8 = 1;

    fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.ts_ms.to_le_bytes());
        buf.extend_from_slice(&self.device_id.to_le_bytes());
        buf.extend_from_slice(&[0u8; 2]); // pad for f32 alignment
        buf.extend_from_slice(&self.temperature.to_le_bytes());
        buf.extend_from_slice(&self.humidity.to_le_bytes());
    }

    fn encoded_size(&self) -> usize { 20 }

    fn decode(data: &[u8]) -> Option<(Self, usize)> {
        if data.len() < 20 { return None; }
        Some((Self {
            ts_ms: i64::from_le_bytes(data[0..8].try_into().ok()?),
            device_id: u16::from_le_bytes(data[8..10].try_into().ok()?),
            temperature: f32::from_le_bytes(data[12..16].try_into().ok()?),
            humidity: f32::from_le_bytes(data[16..20].try_into().ok()?),
        }, 20))
    }
}

fn main() {
    let stream = StreamBuffer::new();

    // Simulate 3 sensors pushing readings
    for device in 0..3u16 {
        stream.push_one(&SensorReading {
            ts_ms: 1_700_000_000_000 + device as i64,
            device_id: device,
            temperature: 22.5 + device as f32 * 0.3,
            humidity: 45.0 + device as f32 * 2.0,
        });
    }

    let bytes = stream.drain();
    println!("Wire: {} bytes for 3 readings", bytes.len());

    let mut reader = FrameReader::new(&bytes);
    while let Some(frame) = reader.next_frame() {
        for reading in frame.decode_all::<SensorReading>().unwrap() {
            println!(
                "  Device {}: {:.1}°C, {:.1}% humidity",
                reading.device_id, reading.temperature, reading.humidity,
            );
        }
    }
}
