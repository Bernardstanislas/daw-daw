// AudioWorklet hosting the daw-daw Rust engine (crates/daw-wasm) in the browser.
// The main thread compiles the .wasm and sends the WebAssembly.Module; this
// processor instantiates it synchronously and renders every audio quantum.
// Strings never get decoded here (TextDecoder is not available in worklets):
// JSON travels as raw UTF-8 bytes in both directions.

const STATUS_EVERY_SEC = 0.03;

class DawEngineProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.x = null;
    this.sinceStatus = 0;
    this.spentMs = 0;
    this.loadFrames = 0;
    this.port.onmessage = (e) => this.onMessage(e.data);
  }

  /** Copies bytes into wasm memory, calls fn(ptr, len), frees. */
  withBytes(bytes, fn) {
    const x = this.x;
    const len = bytes.length;
    const ptr = x.dd_alloc(len);
    new Uint8Array(x.memory.buffer, ptr, len).set(bytes);
    const r = fn(ptr, len);
    x.dd_free(ptr, len);
    return r;
  }

  onMessage(m) {
    if (m.type === "init") {
      try {
        const inst = new WebAssembly.Instance(m.module, {});
        this.x = inst.exports;
        this.x.dd_init(sampleRate);
        this.port.postMessage({ type: "ready", sampleRate });
      } catch (err) {
        this.port.postMessage({ type: "error", message: String(err) });
      }
      return;
    }
    const x = this.x;
    if (!x) return;
    switch (m.type) {
      case "sync":
        this.withBytes(m.bytes, (p, l) => x.dd_sync(p, l));
        break;
      case "transport":
        x.dd_transport(m.cmd, m.position || 0);
        break;
      case "note":
        this.withBytes(m.track || new Uint8Array(0), (p, l) => x.dd_live_note(m.pitch, m.velocity, m.on ? 1 : 0, p, l));
        break;
      case "target":
        this.withBytes(m.track, (p, l) => x.dd_set_live_target(p, l));
        break;
      case "samples": {
        // [{ path: UTF-8 bytes, data: WAV bytes }]
        for (const f of m.files) {
          this.withBytes(f.path, (pp, pl) => this.withBytes(f.data, (dp, dl) => x.dd_add_sample(pp, pl, dp, dl)));
        }
        x.dd_load_samples();
        break;
      }
      case "asset": {
        // planar Float32Array, channels × frames
        const data = m.data;
        const bytes = data.length * 4;
        const dp = x.dd_alloc(bytes);
        new Float32Array(x.memory.buffer, dp, data.length).set(data);
        this.withBytes(m.id, (p, l) => x.dd_add_asset(p, l, m.sampleRate, m.channels, m.frames, dp));
        x.dd_free(dp, bytes);
        break;
      }
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const x = this.x;
    if (!x || !out || out.length === 0) return true;
    const n = out[0].length;
    const w0 = Date.now();
    const ptr = x.dd_process(n);
    const buf = new Float32Array(x.memory.buffer, ptr, n * 2);
    const l = out[0];
    const r = out[1] || out[0];
    if (out.length > 1) {
      for (let i = 0; i < n; i++) {
        l[i] = buf[2 * i];
        r[i] = buf[2 * i + 1];
      }
    } else {
      for (let i = 0; i < n; i++) l[i] = 0.5 * (buf[2 * i] + buf[2 * i + 1]);
    }
    // Date.now() is coarse (1 ms) and a quantum lasts ~3 ms: accumulate over the status period.
    this.spentMs += Date.now() - w0;
    this.loadFrames += n;

    this.sinceStatus += n / sampleRate;
    if (this.sinceStatus >= STATUS_EVERY_SEC) {
      this.sinceStatus = 0;
      x.dd_report_load(this.spentMs, this.loadFrames);
      this.spentMs = 0;
      this.loadFrames = 0;
      const st = Array.from(new Float64Array(x.memory.buffer, x.dd_status(), 8));
      const pp = x.dd_poll();
      const extra = new Uint8Array(x.memory.buffer, pp, x.dd_result_len()).slice();
      this.port.postMessage({ type: "status", status: st, extra }, [extra.buffer]);
    }
    return true;
  }
}

registerProcessor("daw-engine", DawEngineProcessor);
