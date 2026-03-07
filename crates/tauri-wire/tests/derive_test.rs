#![cfg(feature = "derive")]

use tauri_wire::{WireMessage, FrameReader};

#[derive(WireMessage, Debug, PartialEq)]
#[wire(msg_type = 1)]
struct Tick {
    ts_ms: i64,
    price: f64,
    volume: f64,
    side: u8,
}

#[derive(WireMessage, Debug, PartialEq)]
#[wire(msg_type = 2)]
struct Sensor {
    id: u32,
    value: f32,
    flags: u16,
    active: bool,
}

#[test]
fn derive_roundtrip() {
    let tick = Tick {
        ts_ms: 1_700_000_000_000,
        price: 42_000.50,
        volume: 1.25,
        side: 1,
    };

    assert_eq!(tick.encoded_size(), 8 + 8 + 8 + 1);
    assert_eq!(Tick::MSG_TYPE, 1);

    let frame = tauri_wire::encode_frame(&[tick]);
    let mut reader = FrameReader::new(&frame);
    let view = reader.next_frame().unwrap();
    let items = view.decode_all::<Tick>().unwrap();

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].ts_ms, 1_700_000_000_000);
    assert_eq!(items[0].price, 42_000.50);
    assert_eq!(items[0].volume, 1.25);
    assert_eq!(items[0].side, 1);
}

#[test]
fn derive_sensor_roundtrip() {
    let sensor = Sensor {
        id: 42,
        value: 3.14,
        flags: 0xFF00,
        active: true,
    };

    assert_eq!(sensor.encoded_size(), 4 + 4 + 2 + 1);
    assert_eq!(Sensor::MSG_TYPE, 2);

    let mut buf = Vec::new();
    sensor.encode(&mut buf);
    let (decoded, consumed) = Sensor::decode(&buf).unwrap();
    assert_eq!(consumed, 11);
    assert_eq!(decoded.id, 42);
    assert!((decoded.value - 3.14).abs() < 0.001);
    assert_eq!(decoded.flags, 0xFF00);
    assert!(decoded.active);
}

#[test]
fn derive_bool_false() {
    let sensor = Sensor {
        id: 0,
        value: 0.0,
        flags: 0,
        active: false,
    };

    let mut buf = Vec::new();
    sensor.encode(&mut buf);
    let (decoded, _) = Sensor::decode(&buf).unwrap();
    assert!(!decoded.active);
}

#[test]
fn derive_batch_encode_decode() {
    let ticks = vec![
        Tick { ts_ms: 1, price: 1.0, volume: 1.0, side: 0 },
        Tick { ts_ms: 2, price: 2.0, volume: 2.0, side: 1 },
        Tick { ts_ms: 3, price: 3.0, volume: 3.0, side: 0 },
    ];

    let frame = tauri_wire::encode_frame(&ticks);
    let mut reader = FrameReader::new(&frame);
    let view = reader.next_frame().unwrap();
    let items = view.decode_all::<Tick>().unwrap();

    assert_eq!(items.len(), 3);
    assert_eq!(items[0].ts_ms, 1);
    assert_eq!(items[2].ts_ms, 3);
}
