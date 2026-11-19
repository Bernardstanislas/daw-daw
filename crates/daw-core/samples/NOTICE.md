# Samples

Instrument samples (`instruments/`) and the TightKit drum samples (`drums/`) come from the
HiChord 3.0 sound set. They are included at the request of the project owner, who states they
hold the rights to use them in this project. Do not redistribute them separately from this
project without checking those rights.

Root notes were measured from the files (see `tools/measure_pitch.py`); loop points, crossfades,
gains and default envelopes come from the HiChord documentation. Regenerate `manifest.json` and
`src/dsp/sample_data.rs` with `tools/import_hichord.py`.
