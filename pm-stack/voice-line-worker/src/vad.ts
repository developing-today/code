// Voice-activity detection + silence trim for the voice line (#9800).
//
// Pure functions over PCM16LE mono audio — no Cloudflare runtime types —
// so the trim is unit-testable anywhere (bun test). The contract from
// the issue: incoming audio is trimmed / VAD-detected BEFORE it is sent
// for transcription, so silence is never billed (Workers AI transcription
// charges per audio minute submitted, pauses included) and never
// transcribed. Head/tail silence is cut and long internal silences are
// collapsed to a short gap; seconds in vs seconds submitted are both
// reported so the session ledger can prove the saving.

export interface VadOptions {
  /** Samples per analysis frame (320 = 20 ms at 16 kHz). */
  frameSamples?: number;
  /** Voiced if frame RMS >= peakRms * ratio (and >= absoluteFloor). */
  peakRatio?: number;
  /** Absolute RMS floor (PCM16 scale) below which a frame is silence
   *  even if the peak is quiet too. */
  absoluteFloor?: number;
  /** Voiced frames of padding kept before the first / after the last
   *  voiced frame, so onsets and tails are not clipped. */
  padFrames?: number;
  /** Internal silence longer than this (frames) collapses to
   *  collapseToFrames frames. 0 disables collapsing. */
  collapseAfterFrames?: number;
  collapseToFrames?: number;
}

export interface VadResult {
  /** Trimmed PCM16LE bytes, ready to submit for transcription. */
  audio: Uint8Array;
  /** Duration of the submitted (trimmed) audio, seconds. */
  secondsSubmitted: number;
  /** Voiced frames detected in the original audio. */
  voicedFrames: number;
  /** Total frames in the original audio. */
  totalFrames: number;
  /** True when no frame passed the voice threshold — submit nothing. */
  allSilence: boolean;
}

const DEFAULTS: Required<VadOptions> = {
  frameSamples: 320,
  peakRatio: 0.08,
  absoluteFloor: 300,
  padFrames: 5,
  collapseAfterFrames: 30, // 600 ms at 20 ms frames
  collapseToFrames: 10, // 200 ms
};

export function frameRms(samples: Int16Array, start: number, len: number): number {
  let sum = 0;
  const end = Math.min(start + len, samples.length);
  for (let i = start; i < end; i++) sum += samples[i] * samples[i];
  const n = end - start;
  return n > 0 ? Math.sqrt(sum / n) : 0;
}

export function trimSilence(pcm: Uint8Array, sampleRate: number, options: VadOptions = {}): VadResult {
  const opts = { ...DEFAULTS, ...options };
  const samples = new Int16Array(pcm.buffer, pcm.byteOffset, Math.floor(pcm.byteLength / 2));
  const frame = opts.frameSamples;
  const totalFrames = Math.ceil(samples.length / frame);
  const seconds = (frames: number) => (frames * frame) / sampleRate;

  if (totalFrames === 0) {
    return { audio: new Uint8Array(0), secondsSubmitted: 0, voicedFrames: 0, totalFrames: 0, allSilence: true };
  }

  const rms: number[] = new Array(totalFrames);
  let peak = 0;
  for (let f = 0; f < totalFrames; f++) {
    rms[f] = frameRms(samples, f * frame, frame);
    if (rms[f] > peak) peak = rms[f];
  }
  const threshold = Math.max(peak * opts.peakRatio, opts.absoluteFloor);
  const voiced = rms.map((r) => r >= threshold);
  const voicedFrames = voiced.filter(Boolean).length;

  if (voicedFrames === 0) {
    return {
      audio: new Uint8Array(0),
      secondsSubmitted: 0,
      voicedFrames: 0,
      totalFrames,
      allSilence: true,
    };
  }

  const first = voiced.indexOf(true);
  const last = voiced.lastIndexOf(true);
  const keepStart = Math.max(0, first - opts.padFrames);
  const keepEnd = Math.min(totalFrames - 1, last + opts.padFrames);

  // Build the kept frame list, collapsing long internal silences.
  const kept: number[] = [];
  let silenceRun = 0;
  for (let f = keepStart; f <= keepEnd; f++) {
    if (voiced[f]) {
      if (silenceRun > 0 && opts.collapseAfterFrames > 0 && silenceRun > opts.collapseAfterFrames) {
        for (let i = 0; i < opts.collapseToFrames; i++) kept.push(-1);
      } else {
        for (let i = 0; i < silenceRun; i++) kept.push(f - silenceRun + i);
      }
      silenceRun = 0;
      kept.push(f);
    } else {
      silenceRun++;
    }
  }

  const outSamples = kept.length * frame;
  const out = new Int16Array(outSamples);
  kept.forEach((srcFrame, i) => {
    if (srcFrame >= 0) {
      const src = samples.subarray(srcFrame * frame, srcFrame * frame + frame);
      out.set(src, i * frame);
    }
  });

  return {
    audio: new Uint8Array(out.buffer),
    secondsSubmitted: seconds(kept.length),
    voicedFrames,
    totalFrames,
    allSilence: false,
  };
}

/** Duration in seconds of raw PCM16LE audio at a sample rate. */
export function pcmSeconds(byteLength: number, sampleRate: number): number {
  return byteLength / 2 / sampleRate;
}
