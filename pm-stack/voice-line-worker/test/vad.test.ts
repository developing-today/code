import { describe, expect, test } from "bun:test";
import { trimSilence, pcmSeconds } from "../src/vad";

const RATE = 16000;

function pcm(...segments: Array<{ seconds: number; amplitude: number }>): Uint8Array {
  const total = segments.reduce((n, s) => n + Math.round(s.seconds * RATE), 0);
  const out = new Int16Array(total);
  let at = 0;
  for (const s of segments) {
    const n = Math.round(s.seconds * RATE);
    for (let i = 0; i < n; i++) out[at + i] = Math.round(s.amplitude * Math.sin(i / 4) * 32767);
    at += n;
  }
  return new Uint8Array(out.buffer);
}

describe("trimSilence", () => {
  test("cuts head and tail silence, keeps the speech", () => {
    const input = pcm({ seconds: 1.0, amplitude: 0 }, { seconds: 0.5, amplitude: 0.5 }, { seconds: 1.0, amplitude: 0 });
    const r = trimSilence(input, RATE);
    expect(r.allSilence).toBe(false);
    // 0.5 s of speech + at most 0.2 s of padding (5 frames each side).
    expect(r.secondsSubmitted).toBeGreaterThanOrEqual(0.5);
    expect(r.secondsSubmitted).toBeLessThan(0.75);
    expect(pcmSeconds(input.byteLength, RATE)).toBeCloseTo(2.5, 1);
  });

  test("all silence submits nothing", () => {
    const r = trimSilence(pcm({ seconds: 1, amplitude: 0 }), RATE);
    expect(r.allSilence).toBe(true);
    expect(r.audio.byteLength).toBe(0);
    expect(r.secondsSubmitted).toBe(0);
  });

  test("collapses a long internal pause", () => {
    const input = pcm(
      { seconds: 0.4, amplitude: 0.5 },
      { seconds: 2.0, amplitude: 0 },
      { seconds: 0.4, amplitude: 0.5 },
    );
    const r = trimSilence(input, RATE);
    // Without collapsing this would be ~2.8-3.0 s submitted.
    expect(r.secondsSubmitted).toBeLessThan(1.6);
    expect(r.secondsSubmitted).toBeGreaterThan(0.7);
  });

  test("empty input is all silence", () => {
    const r = trimSilence(new Uint8Array(0), RATE);
    expect(r.allSilence).toBe(true);
  });
});
