# gc-dspadpcm

A Python library for encoding mono 16-bit PCM audio to Nintendo Switch BFSTM format using DSP-ADPCM compression. Implemented in Rust via PyO3.

The DSP-ADPCM codec is a direct port of [jackoalan/gc-dspadpcm-encode](https://github.com/jackoalan/gc-dspadpcm-encode) (MIT).

## Installation

Requires Python ≥ 3.10 and a Rust toolchain.

```bash
pip install .
```

## Usage

```python
import gc_dspadpcm

# From a WAV file (mono, 16-bit PCM)
bfstm_bytes = gc_dspadpcm.encode_wav(open("audio.wav", "rb").read())
open("audio.bfstm", "wb").write(bfstm_bytes)

# From raw i16 LE PCM bytes
bfstm_bytes = gc_dspadpcm.encode_pcm(pcm_bytes, sample_rate=22050)
```

### `encode_wav(wav_data: bytes) -> bytes`

Parses a WAV file and encodes it to BFSTM. Accepts mono, 16-bit PCM WAV only — raises `ValueError` for stereo or non-16-bit input.

### `encode_pcm(pcm_data: bytes, sample_rate: int) -> bytes`

Encodes raw mono 16-bit little-endian PCM samples to BFSTM.

## Development

```bash
# Install dev dependencies and build the extension
uv sync

# Force a rebuild after changing Rust source
uv sync --reinstall-package gc-dspadpcm

# Run Rust tests
cargo test

# Run Python tests
uv run pytest tests/python -v
```

Requires Python 3.12.

## Project structure

```
src/
  codec.rs   # DSP-ADPCM encoder (port of grok.c)
  bfstm.rs   # BFSTM container builder
  lib.rs     # PyO3 bindings
tests/
  codec.rs          # Rust unit tests
  bfstm.rs          # Rust integration tests
  python/
    test_gc_dspadpcm.py
```

## License

MIT

# Disclaimer

This project has been created using Claude Code.