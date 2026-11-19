"""Measures the fundamental of each HiChord instrument sample over its loop region.

Usage: python3 -I measure_pitch.py <re_dir> <samples_dir>
Prints one line per sound: name, file, YIN f0, the two f0 values found in the docs.
"""
import json
import sys
import wave
from pathlib import Path

import numpy as np


def read_wav(path: Path) -> tuple[np.ndarray, int]:
    with wave.open(str(path)) as w:
        sr = w.getframerate()
        ch = w.getnchannels()
        width = w.getsampwidth()
        raw = w.readframes(w.getnframes())
    if width != 2:
        raise ValueError(f"{path}: {width * 8}-bit not handled")
    x = np.frombuffer(raw, dtype="<i2").astype(np.float64) / 32768.0
    if ch > 1:
        x = x.reshape(-1, ch).mean(axis=1)
    return x, sr


def yin(x: np.ndarray, sr: int, fmin=40.0, fmax=2000.0, threshold=0.12) -> float:
    """Classic YIN on a single (long) frame."""
    n = min(len(x), 8192)
    x = x[:n] - x[:n].mean()
    tau_max = int(sr / fmin)
    tau_min = int(sr / fmax)
    w = n - tau_max
    if w <= 64:
        return float("nan")
    d = np.array([np.sum((x[:w] - x[t : t + w]) ** 2) for t in range(tau_max + 1)])
    cmnd = np.ones_like(d)
    cumsum = np.cumsum(d[1:])
    cmnd[1:] = d[1:] * np.arange(1, len(d)) / np.maximum(cumsum, 1e-12)
    tau = None
    for t in range(tau_min, tau_max):
        if cmnd[t] < threshold:
            while t + 1 < tau_max and cmnd[t + 1] < cmnd[t]:
                t += 1
            tau = t
            break
    if tau is None:
        tau = tau_min + int(np.argmin(cmnd[tau_min:tau_max]))
    # parabolic interpolation
    if 0 < tau < len(cmnd) - 1:
        a, b, c = cmnd[tau - 1], cmnd[tau], cmnd[tau + 1]
        denom = a - 2 * b + c
        if abs(denom) > 1e-12:
            tau = tau + 0.5 * (a - c) / denom
    return sr / tau


def main():
    re_dir = Path(sys.argv[1])
    samples_dir = Path(sys.argv[2])
    sounds = json.load(open(re_dir / "sounds.json"))["sounds"]
    pitch = json.load(open(re_dir / "sounds_tools" / "pitch.json"))
    by_file = {v["file"]: v for v in pitch.values()}
    for s in sounds:
        smp = s.get("sample") or {}
        wav = smp.get("wav")
        if not wav:
            continue
        name = Path(wav.split(" ")[0]).name
        path = samples_dir / name
        if not path.exists():
            path = re_dir / "sounds_tools" / name
        x, sr = read_wav(path)
        a, b = smp["loop_start_frame"], smp["loop_end_frame"]
        f0 = yin(x[a:b], sr)
        doc = by_file.get(name, {})
        print(f"{s['name']:<10} {name:<16} yin={f0:8.2f}  sounds.json={smp.get('measured_f0_hz')}  pitch.json={doc.get('measured_hz')}")


if __name__ == "__main__":
    main()
