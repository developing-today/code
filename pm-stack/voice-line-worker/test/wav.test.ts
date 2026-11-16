// lane-scratch: code-c /tmp/lane-code-c-20261116-0412 task 16482
import { describe, expect, test } from "bun:test";
import { wavEncodePcm16 } from "../src/session";

// Oracle: Whisper (Workers AI) accepts a RIFF/WAVE container and rejects
// headerless PCM with AiError "Invalid input" (code 8001) — verified live
// against @cf/openai/whisper-large-v3-turbo during the #9800 deploy proof
// (task 16482, 2026-11-16). The encoder must produce a standards-shaped
// 44-byte PCM16 mono header and preserve the payload byte-for-byte.
describe("wavEncodePcm16", () => {
  test("wraps PCM in a 44-byte RIFF/WAVE header", () => {
    const pcm = new Uint8Array([0x01, 0x00, 0xff, 0x7f, 0x00, 0x80]);
    const wav = wavEncodePcm16(pcm, 16000);
    expect(wav.byteLength).toBe(44 + pcm.byteLength);
    const ascii = (off: number, n: number) =>
      String.fromCharCode(...wav.subarray(off, off + n));
    expect(ascii(0, 4)).toBe("RIFF");
    expect(ascii(8, 4)).toBe("WAVE");
    expect(ascii(12, 4)).toBe("fmt ");
    expect(ascii(36, 4)).toBe("data");
    const view = new DataView(wav.buffer);
    expect(view.getUint32(4, true)).toBe(36 + pcm.byteLength);
    expect(view.getUint16(20, true)).toBe(1); // PCM format
    expect(view.getUint16(22, true)).toBe(1); // mono
    expect(view.getUint32(24, true)).toBe(16000); // sample rate
    expect(view.getUint32(28, true)).toBe(32000); // byte rate
    expect(view.getUint16(34, true)).toBe(16); // bits per sample
    expect(view.getUint32(40, true)).toBe(pcm.byteLength);
    expect([...wav.subarray(44)]).toEqual([...pcm]);
  });

  test("empty PCM still yields a valid header", () => {
    const wav = wavEncodePcm16(new Uint8Array(0), 16000);
    expect(wav.byteLength).toBe(44);
    expect(new DataView(wav.buffer).getUint32(40, true)).toBe(0);
  });
});
