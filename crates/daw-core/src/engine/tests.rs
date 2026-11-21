//! Offline engine tests: build projects, render, inspect the audio.

use crate::assets::{AssetCache, AudioBuffer};
use crate::model::*;
use crate::project_io::{ExportOptions, render_offline};
use std::collections::HashMap;

fn dev(id: &str, ty: &str) -> DeviceState {
    DeviceState { id: id.into(), type_id: ty.into(), enabled: true, params: HashMap::new(), state: None }
}

fn note(pitch: u8, start: f64, length: f64) -> Note {
    Note { pitch, start, length, velocity: 100.0 }
}

fn base_project() -> Project {
    Project { bpm: 120.0, time_sig_num: 4, time_sig_den: 4, ..Default::default() }
}

fn midi_track(id: &str, ty: &str, clips: Vec<Clip>) -> Track {
    Track { id: id.into(), kind: TrackKind::Instrument, instrument: Some(dev("i", ty)), clips, ..Default::default() }
}

fn midi_clip(start: f64, length: f64, notes: Vec<Note>) -> Clip {
    Clip { id: "c".into(), kind: ClipKind::Midi, start, length, notes, ..Default::default() }
}

fn opts(end: f64) -> ExportOptions {
    ExportOptions { sample_rate: 48_000, bits: 32, start: 0.0, end, tail: 0.5 }
}

fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0, |m, x| m.max(x.abs()))
}

/// Peak in a time window (seconds), interleaved stereo at 48 kHz.
fn window_peak(buf: &[f32], from: f64, to: f64) -> f32 {
    let a = (from * 48_000.0) as usize * 2;
    let b = ((to * 48_000.0) as usize * 2).min(buf.len());
    peak(&buf[a.min(b)..b])
}

#[test]
fn renders_synth_notes_at_the_right_time() {
    let mut p = base_project();
    // At 120 bpm one beat = 0.5 s. Note at beat 2 (1.0 s) lasting 1 beat.
    p.tracks = vec![midi_track("t1", "pulse", vec![midi_clip(0.0, 4.0, vec![note(60, 2.0, 1.0)])])];
    let out = render_offline(&p, &AssetCache::default(), opts(4.0), |_| {});
    assert!(out.iter().all(|x| x.is_finite()));
    assert!(window_peak(&out, 0.0, 0.95) < 1e-4, "silence before the note");
    assert!(window_peak(&out, 1.0, 1.4) > 0.05, "note sounds");
}

fn rms(buf: &[f32]) -> f32 {
    (buf.iter().map(|x| x * x).sum::<f32>() / buf.len().max(1) as f32).sqrt()
}

#[test]
fn mute_and_solo() {
    let mut p = base_project();
    let mut a = midi_track("a", "pulse", vec![midi_clip(0.0, 4.0, vec![note(60, 0.0, 2.0)])]);
    let b = midi_track("b", "keys", vec![midi_clip(0.0, 4.0, vec![note(64, 0.0, 2.0)])]);
    a.mute = true;
    p.tracks = vec![a.clone()];
    let muted = render_offline(&p, &AssetCache::default(), opts(2.0), |_| {});
    assert!(peak(&muted) < 1e-5, "muted track is silent");

    p.tracks = vec![b.clone()];
    let b_alone = render_offline(&p, &AssetCache::default(), opts(2.0), |_| {});
    a.mute = false;
    let mut b2 = b.clone();
    b2.solo = true;
    p.tracks = vec![a, b2];
    let solo_b = render_offline(&p, &AssetCache::default(), opts(2.0), |_| {});
    let (r1, r2) = (rms(&b_alone), rms(&solo_b));
    assert!(r1 > 0.01 && (r1 - r2).abs() / r1 < 0.1, "solo isolates b: {r1} vs {r2}");
}

#[test]
fn looped_clip_repeats_content() {
    let mut p = base_project();
    let mut c = midi_clip(0.0, 8.0, vec![note(36, 0.0, 0.25)]);
    c.loop_length = 2.0; // kick every 2 beats = every second
    p.tracks = vec![midi_track("d", "kit", vec![c])];
    let out = render_offline(&p, &AssetCache::default(), opts(8.0), |_| {});
    for k in 0..4 {
        let t = k as f64;
        assert!(window_peak(&out, t, t + 0.1) > 0.05, "kick at {t}s");
    }
    let notes = crate::engine::controller::Controller::build_notes(&p.tracks[0].clips);
    assert_eq!(notes.len(), 4);
}

#[test]
fn audio_clip_plays_with_offset() {
    let mut cache = AssetCache::default();
    // 2 s of 0.5 DC at 24 kHz (exercises resampling), second half silent.
    let mut data = vec![0.5f32; 24_000];
    data.extend(vec![0.0; 24_000]);
    cache.insert("a1", "/tmp/x.wav", AudioBuffer { sample_rate: 24_000.0, channels: vec![data] });
    let mut p = base_project();
    p.tracks = vec![Track {
        id: "au".into(),
        kind: TrackKind::Audio,
        clips: vec![Clip {
            id: "c".into(),
            kind: ClipKind::Audio,
            start: 2.0, // 1.0 s
            length: 2.0,
            asset_id: Some("a1".into()),
            offset: 0.5, // start half-way into the DC part
            ..Default::default()
        }],
        ..Default::default()
    }];
    let out = render_offline(&p, &cache, opts(4.0), |_| {});
    assert!(window_peak(&out, 0.0, 0.99) < 1e-4);
    let mid = out[(1.25 * 48_000.0) as usize * 2];
    assert!((mid - 0.5).abs() < 0.02, "dc level through equal-power center pan ≈ 0.5, got {mid}");
    assert!(window_peak(&out, 1.55, 1.95) < 1e-3, "silent part of the file");
    assert!(window_peak(&out, 2.05, 2.4) < 1e-4, "silence after the clip end");
}

#[test]
fn effects_chain_and_master() {
    let mut p = base_project();
    let mut t = midi_track("t", "pulse", vec![midi_clip(0.0, 4.0, vec![note(48, 0.0, 1.0)])]);
    t.effects = vec![dev("e1", "reverb"), dev("e2", "delay"), dev("e3", "unknown-fx")];
    p.tracks = vec![t];
    p.master.effects = vec![dev("m1", "limiter")];
    p.master.volume = -6.0;
    let out = render_offline(&p, &AssetCache::default(), opts(4.0), |_| {});
    assert!(out.iter().all(|x| x.is_finite()));
    assert!(peak(&out) > 0.01 && peak(&out) <= 1.0);
    // reverb/delay tail after the note
    assert!(window_peak(&out, 0.8, 1.6) > 1e-3);
}

#[test]
fn sync_diff_updates_without_rebuild() {
    let (mut c, mut e) = crate::engine::controller::Controller::new(48_000.0);
    let mut p = base_project();
    p.tracks = vec![midi_track("t", "pulse", vec![])];
    c.sync(&p);
    let mut buf = vec![0f32; 1024];
    e.process(&mut buf, 2);
    // change volume, params, clips, then remove the track
    p.tracks[0].volume = -12.0;
    p.tracks[0].instrument.as_mut().unwrap().params.insert("cutoff".into(), 500.0);
    p.tracks[0].clips.push(midi_clip(0.0, 4.0, vec![note(60, 0.0, 1.0)]));
    c.sync(&p);
    e.process(&mut buf, 2);
    p.tracks.clear();
    c.sync(&p);
    e.process(&mut buf, 2);
    c.collect_garbage();
}

#[test]
fn volume_automation_fades_track() {
    let mut p = base_project();
    let mut t = midi_track("t", "keys", vec![midi_clip(0.0, 8.0, vec![note(60, 0.0, 8.0), note(64, 0.0, 8.0)])]);
    // Full level for the first second, silent from beat 4 (2 s) on.
    t.automation.volume = vec![AutoPoint { beat: 0.0, value: 0.0 }, AutoPoint { beat: 2.0, value: 0.0 }, AutoPoint { beat: 4.0, value: -96.0 }];
    p.tracks = vec![t];
    let out = render_offline(&p, &AssetCache::default(), opts(8.0), |_| {});
    assert!(window_peak(&out, 0.1, 0.9) > 0.02);
    assert!(window_peak(&out, 2.2, 3.5) < 1e-4, "automated to silence");
    assert_eq!(crate::engine::track::envelope_at(&[(0.0, 1.0), (2.0, 3.0)], 1.0), Some(2.0));
    assert_eq!(crate::engine::track::envelope_at(&[], 1.0), None);
}

#[test]
fn count_in_then_record() {
    use std::sync::atomic::Ordering;
    let (mut c, mut e) = crate::engine::controller::Controller::new(48_000.0);
    let mut p = base_project();
    p.metronome.count_in_bars = 1;
    c.sync(&p);
    c.send(crate::engine::commands::Command::Seek(8.0));
    c.send(crate::engine::commands::Command::Record);
    let mut buf = vec![0f32; 512 * 2];
    e.process(&mut buf, 2);
    assert!(c.shared.count_in.load(Ordering::Relaxed));
    assert!(!c.shared.recording.load(Ordering::Relaxed));
    assert!(peak(&buf) > 0.01, "count-in click on the first beat");
    // 4 beats at 120 bpm = 2 s = 96000 frames.
    let mut frames = 512;
    while frames < 96_000 - 512 {
        e.process(&mut buf, 2);
        frames += 512;
    }
    assert!(!c.shared.recording.load(Ordering::Relaxed), "still counting in at {frames}");
    for _ in 0..4 {
        e.process(&mut buf, 2);
    }
    assert!(c.shared.recording.load(Ordering::Relaxed), "recording after the count-in");
    assert!((c.shared.record_start.load() - 8.0).abs() < 1e-9);
    assert!(c.shared.position.load() > 8.0 && c.shared.position.load() < 8.2);
}

#[test]
fn count_in_always_ends() {
    use std::sync::atomic::Ordering;
    for &(sr, bpm) in &[(48_000.0, 60.0), (48_000.0, 72.0), (48_000.0, 150.0), (44_100.0, 128.0), (44_100.0, 64.0), (44_100.0, 97.0)] {
        for start in [0.0, 3.0, 8.5] {
            let (mut c, mut e) = crate::engine::controller::Controller::new(sr as f32);
            let mut p = base_project();
            p.bpm = bpm;
            p.metronome.count_in_bars = 1;
            c.sync(&p);
            c.send(crate::engine::commands::Command::Seek(start));
            c.send(crate::engine::commands::Command::Record);
            let mut buf = vec![0f32; 256 * 2];
            let frames = (4.0 * 60.0 / bpm * sr) as usize + 2048;
            let mut done = 0;
            while done < frames {
                e.process(&mut buf, 2);
                done += 256;
            }
            assert!(c.shared.recording.load(Ordering::Relaxed), "recording started at {sr}/{bpm}/{start}");
            assert!(!c.shared.count_in.load(Ordering::Relaxed));
        }
    }
}

#[test]
fn note_at_loop_end_is_not_stuck() {
    for &(sr, bpm) in &[(44_100u32, 120.0), (48_000, 120.0), (48_000, 97.0), (44_100, 140.0)] {
        let mut p = base_project();
        p.bpm = bpm;
        p.loop_region = LoopRegion { enabled: true, start: 0.0, end: 8.0, overdub: false };
        // Only note: exactly on the loop end → must never sound while looping.
        p.tracks = vec![midi_track("t", "pulse", vec![midi_clip(0.0, 16.0, vec![note(60, 8.0, 1.0)])])];
        let (mut c, mut e) = crate::engine::controller::Controller::new(sr as f32);
        c.sync(&p);
        c.send(crate::engine::commands::Command::Play);
        let mut buf = vec![0f32; 512 * 2];
        let frames = (3.0 * 8.0 * 60.0 / bpm * sr as f64) as usize;
        let mut done = 0;
        let mut loudest = 0f32;
        while done < frames {
            e.process(&mut buf, 2);
            loudest = loudest.max(peak(&buf));
            done += 512;
            c.collect_garbage();
        }
        assert!(loudest < 1e-4, "stuck voice at {sr}/{bpm}: {loudest}");
    }
}

#[test]
fn looped_notes_do_not_ring_into_next_pass() {
    let mut c = midi_clip(0.0, 8.0, vec![note(60, 1.0, 5.0)]);
    c.loop_length = 4.0;
    let notes = crate::engine::controller::Controller::build_notes(&[c]);
    assert_eq!(notes.len(), 2);
    assert!((notes[0].end - 4.0).abs() < 1e-9 && (notes[1].end - 8.0).abs() < 1e-9);
}

/// Writes the device catalog for the browser-only UI mock.
/// Run with: cargo test --lib dump_catalog -- --ignored
#[test]
#[ignore]
fn dump_catalog() {
    let json = serde_json::to_string_pretty(&crate::dsp::registry::all_specs()).unwrap();
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/lib/catalog.fixture.json"), json).unwrap();
}

#[test]
fn midi_channel_routes_notes_to_its_track() {
    let (mut c, mut e) = crate::engine::controller::Controller::new(48_000.0);
    let mut p = base_project();
    let mut drums = midi_track("drums", "pulse", vec![]);
    drums.midi_channel = Some(10);
    p.tracks = vec![midi_track("keys", "pulse", vec![]), drums];
    c.sync(&p);
    c.set_live_target(Some("keys".into()));
    let mut buf = vec![0f32; 512 * 2];
    e.process(&mut buf, 2);
    // Channel 10 → drums, even though "keys" is the live target.
    c.midi_note(10, 60, 1.0, true);
    for _ in 0..8 {
        e.process(&mut buf, 2);
    }
    let meters: HashMap<String, (f32, f32)> = c.track_meters().into_iter().map(|(id, l, r)| (id, (l, r))).collect();
    assert!(meters["drums"].0 > 0.01, "drums play channel 10");
    assert!(meters["keys"].0 < 1e-4, "keys stay silent");
    // Channel 1 has no track: it goes to the live target.
    c.midi_note(10, 60, 0.0, false);
    c.midi_note(1, 64, 1.0, true);
    for _ in 0..8 {
        e.process(&mut buf, 2);
    }
    let meters: HashMap<String, (f32, f32)> = c.track_meters().into_iter().map(|(id, l, r)| (id, (l, r))).collect();
    assert!(meters["keys"].0 > 0.01, "unclaimed channel plays the live target");
}

/// Records for 3 s (6 beats at 120 bpm) on a 2-beat loop starting at `start`.
fn record_on_loop(overdub: bool) -> (f64, Vec<f64>) {
    use crate::engine::commands::Command;
    let (mut c, mut e) = crate::engine::controller::Controller::new(48_000.0);
    let mut p = base_project();
    p.metronome.count_in_bars = 0;
    p.loop_region = LoopRegion { enabled: true, start: 4.0, end: 6.0, overdub };
    let mut t = midi_track("t", "pulse", vec![]);
    t.armed = true;
    p.tracks = vec![t];
    c.sync(&p);
    c.set_live_target(Some("t".into()));
    c.send(Command::Seek(4.0));
    c.send(Command::Record);
    let mut buf = vec![0f32; 480 * 2];
    let mut beats = Vec::new();
    // A note every half second (one beat).
    for i in 0..300 {
        if i % 50 == 0 {
            c.midi_note(1, 60, 1.0, true);
        }
        if i % 50 == 25 {
            c.midi_note(1, 60, 0.0, false);
        }
        e.process(&mut buf, 2);
        for (_, ev) in c.drain_recorded() {
            if ev.on {
                beats.push(ev.beat);
            }
        }
    }
    (c.shared.position.load(), beats)
}

#[test]
fn takes_are_linear_unless_the_loop_overdubs() {
    let (pos, beats) = record_on_loop(false);
    assert!(pos > 9.9, "linear take runs past the loop: {pos}");
    assert!(beats.iter().any(|&b| b > 6.0));
    let (pos, beats) = record_on_loop(true);
    assert!((4.0..6.0).contains(&pos), "overdub keeps cycling: {pos}");
    assert_eq!(beats.len(), 6);
    assert!(beats.iter().all(|&b| (4.0..6.0).contains(&b)), "recorded beats stay in the loop: {beats:?}");
}

#[test]
fn punch_out_keeps_playing() {
    use crate::engine::commands::Command;
    use std::sync::atomic::Ordering;
    let (mut c, mut e) = crate::engine::controller::Controller::new(48_000.0);
    let mut p = base_project();
    p.metronome.count_in_bars = 0;
    c.sync(&p);
    c.send(Command::Record);
    let mut buf = vec![0f32; 480 * 2];
    e.process(&mut buf, 2);
    assert!(c.shared.recording.load(Ordering::Relaxed));
    c.send(Command::PunchOut);
    e.process(&mut buf, 2);
    assert!(!c.shared.recording.load(Ordering::Relaxed));
    assert!(c.shared.playing.load(Ordering::Relaxed));
}
