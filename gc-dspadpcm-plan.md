# gc-dspadpcm: Implementation Plan

## Goal

A pip-installable Python library that converts 16-bit PCM audio (supplied as raw bytes)
into a valid Nintendo Switch BFSTM file (bytes), using DSP-ADPCM encoding. Must be
cross-platform (Linux, macOS, Windows) with no .NET dependency.

The Rust crate is named `gc-dspadpcm`; the Python package is `gc_dspadpcm`.

---

## Source reference

The DSP-ADPCM codec is a direct port of:
https://github.com/jackoalan/gc-dspadpcm-encode (MIT licence, two C files)

Only `grok.c` contains the algorithm — `main.c` is a CLI wrapper and is not ported.
Reproduce its two public functions exactly (same integer arithmetic, same floating-point
operations in the same order) to guarantee bit-identical output.

The BFSTM container format spec:
https://nintendo-formats.com/libs/nw/bfstm.html

VGAudio source (BfstmWriter.cs, GcAdpcmEncoder.cs) is a reliable secondary reference
for both the container layout and the DSP seek-chunk format:
https://github.com/Thealexbarney/VGAudio

---

## Repository layout

```
gc-dspadpcm/
├── Cargo.toml
├── pyproject.toml
├── src/
│   ├── lib.rs        # PyO3 module declaration + Python-facing functions
│   ├── codec.rs      # Port of grok.c (all internal helpers + two public fns)
│   └── bfstm.rs      # BFSTM container builder (INFO + SEEK + DATA chunks)
└── tests/
    ├── codec.rs          # Rust unit tests for the DSP-ADPCM codec
    ├── bfstm.rs          # Rust integration tests for full BFSTM output
    └── python/
        ├── conftest.py
        └── test_gc_dspadpcm.py   # pytest tests via the Python bindings
```

---

## Step 1 — Project scaffolding

```bash
maturin init --bindings pyo3 --name gc_dspadpcm
```

### pyproject.toml

```toml
[build-system]
requires = ["maturin>=1.0,<2.0"]
build-backend = "maturin"

[project]
name = "gc-dspadpcm"
version = "0.1.0"
requires-python = ">=3.10"

[tool.maturin]
features = ["pyo3/extension-module"]
module-name = "gc_dspadpcm._lib"
```

### Cargo.toml

```toml
[package]
name = "gc-dspadpcm"
version = "0.1.0"
edition = "2021"

[lib]
name = "gc_dspadpcm"
crate-type = ["cdylib"]

[dependencies]
pyo3 = { version = "0.21", features = ["extension-module"] }

[profile.release]
lto = true
codegen-units = 1
```

---

## Step 2 — Port grok.c to src/codec.rs

### Types

```rust
type TVec = [f64; 3];       // "temporal vector" — index 0 is unused or always 1.0
type CoefPair = [i16; 2];
pub type Coefs = [CoefPair; 8];  // 8 coefficient pairs = 16 i16 values
```

### Helper functions (all `fn`, no `pub`)

Port these verbatim from grok.c, keeping the identical loop bounds and index offsets
(1-based indexing into TVec is intentional in the original):

- `inner_product_merge(vec_out: &mut TVec, pcm_buf: &[i16; 14])`
- `outer_product_merge(mtx_out: &mut [TVec; 3], pcm_buf: &[i16; 14])`
- `analyze_ranges(mtx: &mut [TVec; 3], vec_idxs_out: &mut [usize; 3]) -> bool`
- `bidirectional_filter(mtx: &mut [TVec; 3], vec_idxs: &[usize; 3], vec_out: &mut TVec)`
- `quadratic_merge(in_out_vec: &mut TVec) -> bool`
- `finish_record(in_vec: &mut TVec, out: &mut TVec)`
- `matrix_filter(src: &TVec, dst: &mut TVec)`
- `merge_finish_record(src: &TVec, dst: &mut TVec)`
- `contrast_vectors(source1: &TVec, source2: &TVec) -> f64`
- `filter_records(vec_best: &mut [TVec; 8], exp: usize, records: &[TVec])`

### Public functions

```rust
/// Analyse `samples` (mono 16-bit PCM) and return 8 DSP-ADPCM coefficient pairs.
/// Direct port of DSPCorrelateCoefs from grok.c.
pub fn correlate_coefs(samples: &[i16]) -> Coefs

/// Encode 14 PCM samples (with 2 preceding history samples at index 0 and 1) into
/// one 8-byte DSP-ADPCM frame. Updates pcm_inout in-place with reconstructed samples
/// (caller uses [14] and [15] as next frame's history [0] and [1]).
/// Direct port of DSPEncodeFrame from grok.c.
pub fn encode_frame(
    pcm_inout: &mut [i16; 16],
    sample_count: usize,
    coefs: &Coefs,
) -> [u8; 8]
```

### Full encoding loop (used internally by bfstm.rs)

```rust
pub fn encode_all(samples: &[i16]) -> (Coefs, Vec<u8>) {
    let coefs = correlate_coefs(samples);
    let mut adpcm_frames: Vec<u8> = Vec::new();
    let mut conv_samps = [0i16; 16];
    let packet_count = (samples.len() + 13) / 14;

    for p in 0..packet_count {
        let num_samples = (samples.len() - p * 14).min(14);
        // Fill conv_samps[2..2+num_samples] from the input
        for s in 0..num_samples {
            conv_samps[s + 2] = samples[p * 14 + s];
        }
        // Zero-pad if last frame is short
        for s in num_samples..14 {
            conv_samps[s + 2] = 0;
        }
        let frame = encode_frame(&mut conv_samps, 14, &coefs);
        adpcm_frames.extend_from_slice(&frame);
        // Advance history
        conv_samps[0] = conv_samps[14];
        conv_samps[1] = conv_samps[15];
    }
    (coefs, adpcm_frames)
}
```

Note: `encode_frame` is always called with `sample_count = 14` (even for the last
partial frame, which is zero-padded). This matches the original C behaviour.

---

## Step 3 — Write src/bfstm.rs

Build the BFSTM binary in memory as a `Vec<u8>`. All fields are little-endian unless
noted otherwise. Use the spec at nintendo-formats.com/libs/nw/bfstm.html as the
primary layout reference.

### Key constants for Dread-compatible output

```rust
const BLOCK_SIZE: u32 = 0x2000;         // 8 192 bytes per block
const BLOCK_SAMPLE_COUNT: u32 = 0x3800; // 14 336 samples per block
const SEEK_INTERVAL: u32 = 0x3800;      // one seek entry per block
const ADPCM_CODEC: u8 = 2;             // GC_ADPCM / DSP-ADPCM
```

### Public function

```rust
/// Build a complete, valid BFSTM file from raw mono 16-bit PCM samples.
/// `sample_rate` is in Hz (22 050 for Dread dialogue).
pub fn build_bfstm(samples: &[i16], sample_rate: u32) -> Vec<u8>
```

### BFSTM top-level header (0x40 bytes)

| Offset | Size | Value |
|--------|------|-------|
| 0x00   | 4    | Magic `FSTM` |
| 0x04   | 2    | BOM `0xFF 0xFE` (little-endian marker) |
| 0x06   | 2    | Header size `0x0040` |
| 0x08   | 4    | Version `0x00040006` |
| 0x0C   | 4    | File size (fill in last) |
| 0x10   | 2    | Section count `0x0003` |
| 0x12   | 2    | Padding `0x0000` |
| 0x14   | 8    | INFO section ref: type `0x4000`, padding `0x0000`, offset from file start, size |
| 0x1C   | 8    | SEEK section ref: type `0x4001`, padding `0x0000`, offset, size |
| 0x24   | 8    | DATA section ref: type `0x4002`, padding `0x0000`, offset, size |
| 0x2C   | 20   | Padding to 0x40 |

Each "section ref" is: `u16 type, u16 pad, u32 offset, u32 size`.

### INFO section

Build the INFO chunk per the spec. For mono DSP-ADPCM without looping, the
relevant fields are:

**StreamInfo block (type 0x4100):**
- codec = 2 (ADPCM)
- loop_flag = 0
- channel_count = 1
- region_count = 0
- sample_rate (u32)
- loop_start = 0 (u32)
- sample_count (u32)
- block_count = ceil(sample_count / BLOCK_SAMPLE_COUNT)
- block_size = BLOCK_SIZE
- block_sample_count = BLOCK_SAMPLE_COUNT
- last_block_size = bytes occupied by audio in the final block (without padding)
- last_block_sample_count = sample_count % BLOCK_SAMPLE_COUNT (or BLOCK_SAMPLE_COUNT if exact multiple)
- last_block_padded_size = last_block_size rounded up to next multiple of 0x20
- seek_size = bytes per seek entry × seek entry count (see SEEK section below)
- seek_interval_sample_count = SEEK_INTERVAL
- sample_data reference (type 0x1F00, relative offset into DATA payload)

**Per-channel DspAdpcmInfo (one entry per channel):**
```
coef:           [[i16; 2]; 8]   // 32 bytes — from correlate_coefs()
pred_scale:     i16             // predictor-scale byte of frame 0 = adpcm_frames[0] as i16
yn1:            i16 = 0
yn2:            i16 = 0
loop_pred_scale: i16 = 0
loop_yn1:       i16 = 0
loop_yn2:       i16 = 0
pad:            u16 = 0
// total 46 bytes
```

Reference VGAudio's `BfstmStructure.cs` and `GcAdpcmContext` for the exact byte layout
of the INFO section's channel-info offsets/references, which follow a reference-table
pattern described in the BFSTM spec.

### SEEK section

The SEEK chunk stores decoder context at regular intervals so the engine can seek
without decoding from the beginning.

- One seek entry every SEEK_INTERVAL samples (= one per block).
- Entry count = block_count.
- Each entry = per channel: `(pred_scale: i16, hist1: i16)` = 4 bytes for mono.
  (hist2 is implicitly 0 for all-new blocks, which is adequate for non-looping audio.)

To build the entries: after encoding each block's frames, record the pred_scale from
the first frame of the *next* block and hist values from the last reconstructed sample
of the current block. For the entry at index 0 (before any audio), all values are 0.

Reference: VGAudio `BfstmWriter.cs` → `WriteSEEKChunk` for exact layout.

### DATA section

- Magic `DATA` (4 bytes)
- Section size (u32)
- Padding to 0x20 alignment from section start
- Raw DSP-ADPCM frames — the `adpcm_frames` Vec from `encode_all`, verbatim.

All three sections must begin at offsets aligned to 0x20 bytes from the start of the file.

---

## Step 4 — PyO3 bindings in src/lib.rs

```rust
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// Convert raw mono 16-bit little-endian PCM samples to a BFSTM file.
///
/// Args:
///     pcm_data:    Raw bytes of i16 LE samples (mono).
///     sample_rate: Sample rate in Hz (e.g. 22050).
///
/// Returns:
///     Bytes of a complete BFSTM file.
#[pyfunction]
fn encode_pcm(py: Python<'_>, pcm_data: &[u8], sample_rate: u32) -> PyResult<Py<PyBytes>> {
    // reinterpret pcm_data as &[i16] (LE), call bfstm::build_bfstm
}

/// Parse a minimal PCM WAV file (mono, 16-bit, any sample rate) and encode to BFSTM.
///
/// Validates: RIFF/WAVE header, PCM format (fmt chunk type 1), mono, 16-bit.
/// Returns an error for stereo, float, or non-PCM WAV files.
///
/// Args:
///     wav_data: Bytes of a WAV file.
///
/// Returns:
///     Bytes of a complete BFSTM file.
#[pyfunction]
fn encode_wav(py: Python<'_>, wav_data: &[u8]) -> PyResult<Py<PyBytes>> {
    // parse WAV header to extract sample_rate and PCM samples, then call encode_pcm
}

#[pymodule]
fn gc_dspadpcm(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(encode_pcm, m)?)?;
    m.add_function(wrap_pyfunction!(encode_wav, m)?)?;
    Ok(())
}
```

The WAV parser only needs to handle the minimal case: RIFF header, `fmt ` chunk
(format=1, channels=1, bits_per_sample=16), `data` chunk. Skip unknown chunks.
Reject anything that is not mono 16-bit PCM with a descriptive Python `ValueError`.

---

## Step 5 — Testing

### 5a — Rust unit tests (tests/codec.rs)

**Silence test:** encode 14 zero-valued samples. Verify the output frame is
`[0x00; 8]` (scale 0, coef pair 0, all nibbles 0).

**Known-vector test:** encode the specific 14-sample sequence below and assert the
exact 8-byte output. Derive the expected value by running the original C code
(`gcc grok.c main.c -lm && ./a.out input.wav output.dsp`) on the same input and
reading bytes 0x60–0x67 of the output (the first frame, after the 96-byte header).

**Coefficient round-trip:** call `correlate_coefs` on a short sine wave (e.g. 256
samples at 440 Hz, 22 050 Hz sample rate). Assert all 8 coefficient pairs are within
the i16 range and that the first pair is non-zero.

**History propagation:** encode two consecutive frames and verify that the history
values written into `conv_samps[0]`/`[1]` by the first call are used correctly in
the second, i.e. that `conv_samps[14]` and `conv_samps[15]` after frame 1 equal
`conv_samps[0]` and `conv_samps[1]` before frame 2.

### 5b — Rust integration tests (tests/bfstm.rs)

**Magic and header fields:** call `build_bfstm` on a short sine wave; assert:
- bytes 0–3 == `b"FSTM"`
- bytes 4–5 == `[0xFF, 0xFE]`
- `u16` at offset 0x10 == 3 (section count)
- `u32` at offset 0x0C equals `bfstm.len() as u32`

**Section alignment:** read the three section offsets from the header; assert each
is a multiple of 0x20.

**DATA integrity:** locate the DATA section; assert its magic is `b"DATA"` and that
its payload length equals `ceil(sample_count / 14) * 8` bytes (i.e. one 8-byte frame
per 14 samples, no extra bytes).

**Round-trip via vgmstream (optional, CI-gated):** if `vgmstream-cli` is on PATH,
decode the produced BFSTM back to WAV and assert that the SNR between original and
decoded audio exceeds 20 dB. DSP-ADPCM is lossy but should easily exceed this
threshold for typical speech audio.

### 5c — Python binding tests (tests/python/test_gc_dspadpcm.py)

Use `pytest`. Do not depend on any audio libraries for building test fixtures; generate
PCM data with the `struct` and `wave` standard-library modules only.

```python
import struct, wave, io, gc_dspadpcm

def make_sine_wav(freq=440, duration=0.5, sample_rate=22050) -> bytes:
    """Return bytes of a minimal mono 16-bit PCM WAV."""
    import math
    samples = [int(32767 * math.sin(2 * math.pi * freq * i / sample_rate))
               for i in range(int(sample_rate * duration))]
    buf = io.BytesIO()
    with wave.open(buf, 'wb') as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sample_rate)
        w.writeframes(struct.pack(f'<{len(samples)}h', *samples))
    return buf.getvalue()

def test_encode_wav_returns_bytes():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    assert isinstance(bfstm, bytes)

def test_encode_wav_magic():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    assert bfstm[:4] == b'FSTM'

def test_encode_wav_bom():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    assert bfstm[4:6] == b'\xff\xfe'

def test_encode_wav_file_size_field():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    file_size = struct.unpack_from('<I', bfstm, 0x0C)[0]
    assert file_size == len(bfstm)

def test_encode_wav_section_count():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    assert struct.unpack_from('<H', bfstm, 0x10)[0] == 3

def test_encode_pcm_matches_encode_wav():
    wav = make_sine_wav()
    # Extract raw PCM from the WAV
    with wave.open(io.BytesIO(wav)) as w:
        pcm = w.readframes(w.getnframes())
        sr = w.getframerate()
    assert gc_dspadpcm.encode_wav(wav) == gc_dspadpcm.encode_pcm(pcm, sr)

def test_encode_wav_rejects_stereo():
    import pytest
    buf = io.BytesIO()
    with wave.open(buf, 'wb') as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(22050)
        w.writeframes(b'\x00' * 88200)
    with pytest.raises(ValueError, match="mono"):
        gc_dspadpcm.encode_wav(buf.getvalue())

def test_encode_wav_rejects_8bit():
    import pytest
    buf = io.BytesIO()
    with wave.open(buf, 'wb') as w:
        w.setnchannels(1)
        w.setsampwidth(1)
        w.setframerate(22050)
        w.writeframes(b'\x80' * 22050)
    with pytest.raises(ValueError, match="16-bit"):
        gc_dspadpcm.encode_wav(buf.getvalue())

def test_encode_wav_silence():
    """Silence should produce a valid BFSTM without panicking."""
    buf = io.BytesIO()
    n = 22050
    with wave.open(buf, 'wb') as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(22050)
        w.writeframes(b'\x00' * (n * 2))
    bfstm = gc_dspadpcm.encode_wav(buf.getvalue())
    assert bfstm[:4] == b'FSTM'
```

### 5d — Cross-encoder comparison test (tests/python/test_vgaudio_parity.py, optional)

If VGAudioCli is available on PATH (e.g. in CI via a Docker image), encode the same
WAV with both this library and VGAudio and compare:

1. The 16 DSP-ADPCM coefficients in the INFO chunk must be identical.
2. The encoded ADPCM frames in the DATA section must be identical (bit-exact match).
   VGAudio uses the same reference algorithm; any divergence indicates a porting error.

Gate this test with `pytest.importorskip` / a `skipif` on `shutil.which("VGAudioCli")`.

---

## Step 6 — CI

Add a GitHub Actions workflow with three jobs:

1. **build** — `maturin develop` on ubuntu-latest / macos-latest / windows-latest,
   then `cargo test` and `pytest`.
2. **lint** — `cargo clippy -- -D warnings` and `cargo fmt --check`.
3. **maturin-build** — build release wheels with `maturin build --release` to confirm
   the packaging is correct without uploading.

---

## Acceptance criteria

- `pip install .` succeeds on Linux, macOS, and Windows.
- `gc_dspadpcm.encode_wav(open("diag_adam_aqua_1_page_1_orig.wav","rb").read())` produces
  a file that passes validation by `vgmstream-cli -m` without errors.
- All pytest tests pass.
- All `cargo test` tests pass.
