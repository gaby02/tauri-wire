/**
 * tauri-wire TypeScript decoder.
 *
 * Generic frame iterator for binary streams produced by the Rust side.
 * Reads via DataView — no JSON.parse(), no string allocation.
 *
 * Usage:
 *
 * ```ts
 * import { invoke } from '@tauri-apps/api/core';
 * import { FrameIterator } from 'tauri-wire/decoder';
 *
 * const bytes: ArrayBuffer = await invoke('poll');
 * for (const frame of new FrameIterator(bytes)) {
 *   switch (frame.msgType) {
 *     case 1: handleTicks(frame); break;
 *     case 2: handleBars(frame);  break;
 *     default: break; // skip unknown
 *   }
 * }
 * ```
 */

const FRAME_HEADER_SIZE = 9;

/**
 * A zero-copy view of one frame within a binary buffer.
 *
 * Provides typed read helpers at byte offsets within the payload.
 * All reads are little-endian to match the Rust wire format.
 */
export class Frame {
  /** User-defined message type tag (0-255) */
  readonly msgType: number;
  /** Monotonic sequence number (per msg_type). Gaps indicate dropped frames. */
  readonly sequence: number;
  /** Byte length of the payload */
  readonly payloadBytes: number;

  private readonly view: DataView;
  private readonly payloadOffset: number;

  constructor(view: DataView, offset: number) {
    this.view = view;
    this.msgType = view.getUint8(offset);
    this.sequence = view.getUint32(offset + 1, true);
    this.payloadBytes = view.getUint32(offset + 5, true);
    this.payloadOffset = offset + FRAME_HEADER_SIZE;
  }

  // ── Typed reads at payload-relative offsets ──────────────────

  readI8(offset: number): number {
    return this.view.getInt8(this.payloadOffset + offset);
  }

  readU8(offset: number): number {
    return this.view.getUint8(this.payloadOffset + offset);
  }

  readBool(offset: number): boolean {
    return this.view.getUint8(this.payloadOffset + offset) !== 0;
  }

  readI16(offset: number): number {
    return this.view.getInt16(this.payloadOffset + offset, true);
  }

  readU16(offset: number): number {
    return this.view.getUint16(this.payloadOffset + offset, true);
  }

  readU32(offset: number): number {
    return this.view.getUint32(this.payloadOffset + offset, true);
  }

  readI32(offset: number): number {
    return this.view.getInt32(this.payloadOffset + offset, true);
  }

  readF32(offset: number): number {
    return this.view.getFloat32(this.payloadOffset + offset, true);
  }

  readF64(offset: number): number {
    return this.view.getFloat64(this.payloadOffset + offset, true);
  }

  /**
   * Read an i64 as a JavaScript number.
   * Safe for timestamps up to 2^53 (~285,000 years from epoch).
   */
  readI64(offset: number): number {
    const lo = this.view.getUint32(this.payloadOffset + offset, true);
    const hi = this.view.getInt32(this.payloadOffset + offset + 4, true);
    return hi * 0x1_0000_0000 + lo;
  }

  /**
   * Read a u64 as a JavaScript number.
   * Safe up to Number.MAX_SAFE_INTEGER (2^53 - 1).
   */
  readU64(offset: number): number {
    const lo = this.view.getUint32(this.payloadOffset + offset, true);
    const hi = this.view.getUint32(this.payloadOffset + offset + 4, true);
    return hi * 0x1_0000_0000 + lo;
  }

  /**
   * Get a raw Uint8Array view of a portion of the payload (zero-copy).
   */
  rawBytes(offset: number, length: number): Uint8Array {
    return new Uint8Array(
      this.view.buffer,
      this.view.byteOffset + this.payloadOffset + offset,
      length,
    );
  }

  /**
   * For fixed-size items: iterate over payload in chunks.
   * Calls `fn` with the byte offset of each item within the payload.
   */
  forEachItem(itemSize: number, fn: (payloadOffset: number) => void): void {
    for (let off = 0; off + itemSize <= this.payloadBytes; off += itemSize) {
      fn(off);
    }
  }

  /** Number of fixed-size items in the payload. */
  itemCount(itemSize: number): number {
    return Math.floor(this.payloadBytes / itemSize);
  }
}

/**
 * Iterate over frames in a binary buffer.
 *
 * Implements the iterable protocol, so you can use `for...of`:
 *
 * ```ts
 * for (const frame of new FrameIterator(buffer)) {
 *   console.log(frame.msgType, frame.sequence, frame.payloadBytes);
 * }
 * ```
 */
export class FrameIterator implements IterableIterator<Frame> {
  private readonly view: DataView;
  private offset: number = 0;

  constructor(buffer: ArrayBuffer) {
    this.view = new DataView(buffer);
  }

  next(): IteratorResult<Frame> {
    if (this.offset + FRAME_HEADER_SIZE > this.view.byteLength) {
      return { value: undefined, done: true };
    }

    const frame = new Frame(this.view, this.offset);
    const frameEnd = this.offset + FRAME_HEADER_SIZE + frame.payloadBytes;

    if (frameEnd > this.view.byteLength) {
      return { value: undefined, done: true }; // truncated frame
    }

    this.offset = frameEnd;
    return { value: frame, done: false };
  }

  [Symbol.iterator](): IterableIterator<Frame> {
    return this;
  }

  /** Bytes remaining in the buffer. */
  get remaining(): number {
    return Math.max(0, this.view.byteLength - this.offset);
  }
}

/**
 * Register a typed decoder for a specific message type.
 *
 * This is a convenience pattern — define your decode logic once,
 * then dispatch frames by type in a tight loop.
 *
 * ```ts
 * const decoders = new Map<number, (frame: Frame) => void>();
 *
 * decoders.set(1, (frame) => {
 *   frame.forEachItem(24, (off) => {
 *     const ts    = frame.readI64(off);
 *     const value = frame.readF64(off + 8);
 *     const ch    = frame.readU32(off + 16);
 *     chart.addPoint(ts, value, ch);
 *   });
 * });
 *
 * function onData(buffer: ArrayBuffer) {
 *   for (const frame of new FrameIterator(buffer)) {
 *     decoders.get(frame.msgType)?.(frame);
 *   }
 * }
 * ```
 */
export type FrameDecoder = (frame: Frame) => void;
