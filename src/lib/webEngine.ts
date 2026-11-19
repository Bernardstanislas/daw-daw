// Browser audio: runs the Rust engine (crates/daw-wasm, built to public/daw-engine.wasm)
// inside an AudioWorklet, plays MIDI from Web MIDI keyboards, decodes imported
// files with Web Audio and renders exports offline with a second engine instance.

import type { AssetInfo, EngineStatus, Project } from "./types";

type Listener = (payload: unknown) => void;

const WASM_URL = "/daw-engine.wasm";
const WORKLET_URL = "/engine-worklet.js";
const PEAKS_PER_SECOND = 200;

const enc = new TextEncoder();
const dec = new TextDecoder();

let ctx: AudioContext | null = null;
let node: AudioWorkletNode | null = null;
let wasmModule: WebAssembly.Module | null = null;
let ready = false;
let failure: string | null = null;
const queue: unknown[] = [];
const listeners = new Map<string, Set<Listener>>();
/** Decoded PCM kept for offline export and session reloads. */
const pcm = new Map<string, { sampleRate: number; channels: Float32Array[]; info: AssetInfo }>();
let midiPorts: string[] = [];
let midiAccess: MIDIAccess | null = null;

export function on(event: string, cb: Listener): () => void {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event)!.add(cb);
  return () => listeners.get(event)?.delete(cb);
}

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((cb) => cb(payload));
}

function post(msg: unknown, transfer: Transferable[] = []) {
  if (ready && node) node.port.postMessage(msg, transfer);
  else queue.push(msg);
}

export function webEngineState() {
  return {
    ready,
    failure,
    sampleRate: ctx?.sampleRate ?? 0,
    running: ctx?.state === "running",
    midiPorts,
  };
}

/** Starts Web Audio + the wasm engine. Safe to call several times. */
export async function startWebEngine(): Promise<void> {
  if (ctx) return;
  try {
    ctx = new AudioContext({ latencyHint: "interactive" });
    const res = await fetch(WASM_URL);
    if (!res.ok) throw new Error(`${WASM_URL} not found — run "pnpm build:wasm" (or "pnpm dev:web")`);
    wasmModule = await WebAssembly.compile(await res.arrayBuffer());
    await ctx.audioWorklet.addModule(WORKLET_URL);
    node = new AudioWorkletNode(ctx, "daw-engine", { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2] });
    node.connect(ctx.destination);
    await new Promise<void>((resolve, reject) => {
      node!.port.onmessage = (e) => {
        const m = e.data;
        if (m.type === "ready") resolve();
        else if (m.type === "error") reject(new Error(m.message));
      };
      node!.port.postMessage({ type: "init", module: wasmModule });
    });
    node.port.onmessage = (e) => onWorkletMessage(e.data);
    ready = true;
    for (const m of queue.splice(0)) node.port.postMessage(m);
    // Browsers only start audio after a user gesture.
    const resume = () => {
      if (ctx && ctx.state !== "running") ctx.resume();
    };
    window.addEventListener("pointerdown", resume, true);
    window.addEventListener("keydown", resume, true);
  } catch (e) {
    failure = e instanceof Error ? e.message : String(e);
    console.error("[web engine]", failure);
  }
  initMidi();
}

function onWorkletMessage(m: { type: string; status?: number[]; extra?: Uint8Array }) {
  if (m.type !== "status" || !m.status) return;
  let meters: Record<string, [number, number]> = {};
  let recorded: unknown[] = [];
  try {
    const x = JSON.parse(dec.decode(m.extra));
    meters = x.meters ?? {};
    recorded = x.recorded ?? [];
  } catch {
    /* empty poll */
  }
  // Recorded notes first: a take finishes when a status says recording=false.
  if (recorded.length) emit("midi-recorded", recorded);
  const s = m.status;
  const status: EngineStatus = {
    position: s[0],
    playing: s[1] > 0,
    recording: s[2] > 0,
    countIn: s[3] > 0,
    cpu: s[4],
    master: [s[5], s[6]],
    input: [0, 0],
    meters,
  };
  emit("engine-status", status);
}

// ------------------------------------------------------------------ engine API

export function syncProject(p: Project) {
  post({ type: "sync", bytes: enc.encode(JSON.stringify(p)) });
}

/** 0 play · 1 stop · 2 seek · 3 record · 4 panic */
export function transport(cmd: number, position = 0) {
  if (cmd === 0 || cmd === 3) ctx?.resume();
  post({ type: "transport", cmd, position });
}

export function liveNote(pitch: number, velocity: number, on: boolean, trackId: string | null) {
  ctx?.resume();
  post({ type: "note", pitch, velocity, on, track: trackId ? enc.encode(trackId) : undefined });
}

export function setLiveTarget(trackId: string | null) {
  post({ type: "target", track: enc.encode(trackId ?? "") });
}

function sendAsset(id: string, sampleRate: number, channels: Float32Array[]) {
  const frames = channels[0]?.length ?? 0;
  const data = new Float32Array(frames * channels.length);
  channels.forEach((c, i) => data.set(c, i * frames));
  post({ type: "asset", id: enc.encode(id), sampleRate, channels: channels.length, frames, data }, [data.buffer]);
}

// ------------------------------------------------------------------ files

function peaksOf(channels: Float32Array[], sampleRate: number): number[] {
  const bucket = Math.max(1, sampleRate / PEAKS_PER_SECOND);
  const n = Math.ceil(channels[0].length / bucket);
  const out = new Array<number>(n).fill(0);
  for (const ch of channels) {
    for (let i = 0; i < ch.length; i++) {
      const b = Math.floor(i / bucket);
      const a = Math.abs(ch[i]);
      if (a > out[b]) out[b] = a;
    }
  }
  return out;
}

/** Decodes an audio file with Web Audio and hands it to the engine. */
export async function importFile(id: string, file: File): Promise<AssetInfo> {
  await startWebEngine();
  const ac = ctx ?? new AudioContext();
  const buf = await ac.decodeAudioData(await file.arrayBuffer());
  const channels = Array.from({ length: Math.min(2, buf.numberOfChannels) }, (_, c) => new Float32Array(buf.getChannelData(c)));
  const info: AssetInfo = {
    id,
    path: `browser:${file.name}`,
    name: file.name.replace(/\.[^.]+$/, ""),
    duration: buf.duration,
    channels: channels.length,
    sampleRate: buf.sampleRate,
    peaks: peaksOf(channels, buf.sampleRate),
    peaksPerSecond: PEAKS_PER_SECOND,
  };
  pcm.set(id, { sampleRate: buf.sampleRate, channels, info });
  sendAsset(id, buf.sampleRate, channels.map((c) => c.slice()));
  return info;
}

export function assetInfo(id: string): AssetInfo | null {
  return pcm.get(id)?.info ?? null;
}

// ------------------------------------------------------------------ MIDI

function initMidi() {
  if (midiAccess || !("requestMIDIAccess" in navigator)) return;
  navigator
    .requestMIDIAccess({ sysex: false })
    .then((access) => {
      midiAccess = access;
      const bind = () => {
        midiPorts = [];
        access.inputs.forEach((input) => {
          midiPorts.push(input.name ?? "MIDI input");
          input.onmidimessage = (e) => onMidi(e.data);
        });
        emit("midi-ports", midiPorts);
      };
      bind();
      access.onstatechange = bind;
    })
    .catch((e) => console.warn("[web midi]", e));
}

export function rescanMidi(): string[] {
  if (!midiAccess) initMidi();
  else {
    midiPorts = [];
    midiAccess.inputs.forEach((i) => {
      midiPorts.push(i.name ?? "MIDI input");
      i.onmidimessage = (e) => onMidi(e.data);
    });
  }
  return midiPorts;
}

function onMidi(data: Uint8Array | null) {
  if (!data || data.length < 3) return;
  const status = data[0] & 0xf0;
  const pitch = data[1];
  const vel = data[2];
  if (status === 0x90 && vel > 0) {
    liveNote(pitch, vel / 127, true, null);
    emit("midi-input", [pitch, vel, true]);
  } else if (status === 0x80 || (status === 0x90 && vel === 0)) {
    liveNote(pitch, 0, false, null);
    emit("midi-input", [pitch, 0, false]);
  }
}

// ------------------------------------------------------------------ export

function encodeWav(interleaved: Float32Array, sampleRate: number, bits: number): Blob {
  const channels = 2;
  const bytesPer = bits / 8;
  const dataLen = interleaved.length * bytesPer;
  const buf = new ArrayBuffer(44 + dataLen);
  const v = new DataView(buf);
  const str = (o: number, s: string) => [...s].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  str(0, "RIFF");
  v.setUint32(4, 36 + dataLen, true);
  str(8, "WAVE");
  str(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, bits === 32 ? 3 : 1, true);
  v.setUint16(22, channels, true);
  v.setUint32(24, sampleRate, true);
  v.setUint32(28, sampleRate * channels * bytesPer, true);
  v.setUint16(32, channels * bytesPer, true);
  v.setUint16(34, bits, true);
  str(36, "data");
  v.setUint32(40, dataLen, true);
  let o = 44;
  for (let i = 0; i < interleaved.length; i++) {
    const s = Math.max(-1, Math.min(1, interleaved[i]));
    if (bits === 16) {
      v.setInt16(o, Math.round(s * 32767), true);
    } else if (bits === 24) {
      const x = Math.round(s * 8388607);
      v.setUint8(o, x & 0xff);
      v.setUint8(o + 1, (x >> 8) & 0xff);
      v.setUint8(o + 2, (x >> 16) & 0xff);
    } else {
      v.setFloat32(o, interleaved[i], true);
    }
    o += bytesPer;
  }
  return new Blob([buf], { type: "audio/wav" });
}

/** Renders the project with a fresh engine instance and downloads a WAV. */
export async function exportWav(
  fileName: string,
  project: Project,
  opts: { sampleRate: number; bits: number; start: number; end: number; tail: number },
) {
  if (!wasmModule) await startWebEngine();
  if (!wasmModule) throw new Error(failure ?? "audio engine unavailable");
  const { exports } = await WebAssembly.instantiate(wasmModule, {});
  const x = exports as unknown as {
    memory: WebAssembly.Memory;
    dd_init(sr: number): void;
    dd_alloc(n: number): number;
    dd_free(p: number, n: number): void;
    dd_add_asset(ip: number, il: number, sr: number, ch: number, frames: number, dp: number): void;
    dd_render(p: number, l: number, sr: number, start: number, end: number, tail: number): number;
    dd_result_len(): number;
  };
  x.dd_init(opts.sampleRate);
  const withBytes = <T,>(bytes: Uint8Array, fn: (p: number, l: number) => T): T => {
    const p = x.dd_alloc(bytes.length);
    new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
    const r = fn(p, bytes.length);
    x.dd_free(p, bytes.length);
    return r;
  };
  for (const [id, a] of pcm) {
    const frames = a.channels[0].length;
    const n = frames * a.channels.length;
    const dp = x.dd_alloc(n * 4);
    const view = new Float32Array(x.memory.buffer, dp, n);
    a.channels.forEach((c, i) => view.set(c, i * frames));
    withBytes(enc.encode(id), (p, l) => x.dd_add_asset(p, l, a.sampleRate, a.channels.length, frames, dp));
    x.dd_free(dp, n * 4);
  }
  const ptr = withBytes(enc.encode(JSON.stringify(project)), (p, l) => x.dd_render(p, l, opts.sampleRate, opts.start, opts.end, opts.tail));
  if (!ptr) throw new Error("render failed");
  const samples = new Float32Array(x.memory.buffer, ptr, x.dd_result_len()).slice();
  const blob = encodeWav(samples, opts.sampleRate, opts.bits);
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = fileName.endsWith(".wav") ? fileName : `${fileName}.wav`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 10_000);
}
