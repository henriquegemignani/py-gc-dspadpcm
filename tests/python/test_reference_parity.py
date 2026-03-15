"""Integration tests comparing gc_dspadpcm output to the reference C implementation.

The reference C sources (grok.c + main.c) in tests/reference/ are compiled with gcc
at test-collection time.  If gcc is unavailable the whole module is skipped.

DSP file layout (reference output):
  bytes  0–95  : big-endian header
    offset  28 : 16 × big-endian int16 predictor coefficients
  bytes 96+    : raw ADPCM frames (8 bytes each for full 14-sample frames)

BFSTM coef extraction:
  info_off = u32LE @ 0x18
  coefs    = 16 × s16LE @ info_off + 0x6C

BFSTM ADPCM data extraction:
  data_off = u32LE @ 0x30
  audio    starts at data_off + 0x20
"""
import math
import os
import struct
import subprocess  # used in encode_ref
import wave
import io

import pytest
import gc_dspadpcm

_REF_DIR = os.path.join(os.path.dirname(__file__), "..", "reference")


# ── Helpers ──────────────────────────────────────────────────────────────────

def make_wav(samples: list[int], sample_rate: int = 22050) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sample_rate)
        w.writeframes(struct.pack(f"<{len(samples)}h", *samples))
    return buf.getvalue()


def ref_coefs(dsp: bytes) -> list[int]:
    return list(struct.unpack_from(">16h", dsp, 28))


def ref_adpcm(dsp: bytes, n_full_frames: int) -> bytes:
    return dsp[96: 96 + n_full_frames * 8]


def bfstm_coefs(bfstm: bytes) -> list[int]:
    info_off = struct.unpack_from("<I", bfstm, 0x18)[0]
    return list(struct.unpack_from("<16h", bfstm, info_off + 0x6C))


def bfstm_adpcm(bfstm: bytes, n_bytes: int) -> bytes:
    data_off = struct.unpack_from("<I", bfstm, 0x30)[0]
    start = data_off + 0x20
    return bfstm[start: start + n_bytes]


# ── Fixture: pre-built reference binary ──────────────────────────────────────
# Build it with:  cmake -S tests/reference -B tests/reference/build && cmake --build tests/reference/build
# CI does this automatically before running pytest.

@pytest.fixture(scope="module")
def ref_bin():
    ext = ".exe" if os.name == "nt" else ""
    # cmake --build puts the binary in build/ or build/Debug/ on MSVC
    candidates = [
        os.path.join(_REF_DIR, "build", f"dspadpcm_ref{ext}"),
        os.path.join(_REF_DIR, "build", "Debug", f"dspadpcm_ref{ext}"),
        os.path.join(_REF_DIR, "build", "Release", f"dspadpcm_ref{ext}"),
    ]
    for path in candidates:
        if os.path.isfile(path):
            return path
    pytest.skip(
        "Reference binary not found — build it first:\n"
        "  cmake -S tests/reference -B tests/reference/build\n"
        "  cmake --build tests/reference/build"
    )


def encode_ref(bin_path: str, wav: bytes, tmp_path) -> bytes:
    wav_p = tmp_path / "in.wav"
    dsp_p = tmp_path / "out.dsp"
    wav_p.write_bytes(wav)
    r = subprocess.run([bin_path, str(wav_p), str(dsp_p)],
                       capture_output=True, text=True)
    assert r.returncode == 0, f"Reference encoder failed:\n{r.stderr}"
    return dsp_p.read_bytes()


# ── Tests ─────────────────────────────────────────────────────────────────────

class TestReferenceParity:
    """Each test encodes the same WAV with both the reference C binary and
    gc_dspadpcm, then asserts bit-identical coefficients and ADPCM frames.

    Only sample counts that are exact multiples of 14 are used so the reference
    always writes complete 8-byte frames (no partial-last-frame size mismatch).
    """

    @pytest.fixture(autouse=True)
    def _tmp(self, tmp_path):
        self.tmp = tmp_path

    def _both(self, ref_bin, samples, sample_rate=22050):
        wav = make_wav(samples, sample_rate)
        dsp = encode_ref(ref_bin, wav, self.tmp)
        bfstm = gc_dspadpcm.encode_wav(wav)
        return dsp, bfstm

    # ── Coefficients ─────────────────────────────────────────────────────────

    def test_coefs_sine(self, ref_bin):
        n = 14 * 10
        samples = [int(16000 * math.sin(2 * math.pi * i / 32)) for i in range(n)]
        dsp, bfstm = self._both(ref_bin, samples)
        assert ref_coefs(dsp) == bfstm_coefs(bfstm), "coef mismatch (sine)"

    def test_coefs_sawtooth(self, ref_bin):
        n = 14 * 20
        period = 32
        samples = [int(20000 * (i % period) / period) - 10000 for i in range(n)]
        dsp, bfstm = self._both(ref_bin, samples)
        assert ref_coefs(dsp) == bfstm_coefs(bfstm), "coef mismatch (sawtooth)"

    def test_coefs_silence(self, ref_bin):
        n = 14 * 5
        dsp, bfstm = self._both(ref_bin, [0] * n)
        assert ref_coefs(dsp) == bfstm_coefs(bfstm), "coef mismatch (silence)"

    def test_coefs_single_frame(self, ref_bin):
        samples = [i * 1000 for i in range(-7, 7)]  # 14 samples
        dsp, bfstm = self._both(ref_bin, samples)
        assert ref_coefs(dsp) == bfstm_coefs(bfstm), "coef mismatch (single frame)"

    # ── ADPCM frames ─────────────────────────────────────────────────────────

    def test_adpcm_sine(self, ref_bin):
        n = 14 * 10
        samples = [int(16000 * math.sin(2 * math.pi * i / 32)) for i in range(n)]
        dsp, bfstm = self._both(ref_bin, samples)
        frames = n // 14
        assert ref_adpcm(dsp, frames) == bfstm_adpcm(bfstm, frames * 8), \
            "ADPCM mismatch (sine)"

    def test_adpcm_sawtooth(self, ref_bin):
        n = 14 * 20
        period = 32
        samples = [int(20000 * (i % period) / period) - 10000 for i in range(n)]
        dsp, bfstm = self._both(ref_bin, samples)
        frames = n // 14
        assert ref_adpcm(dsp, frames) == bfstm_adpcm(bfstm, frames * 8), \
            "ADPCM mismatch (sawtooth)"

    def test_adpcm_silence(self, ref_bin):
        n = 14 * 5
        dsp, bfstm = self._both(ref_bin, [0] * n)
        frames = n // 14
        assert ref_adpcm(dsp, frames) == bfstm_adpcm(bfstm, frames * 8), \
            "ADPCM mismatch (silence)"

    def test_adpcm_single_frame(self, ref_bin):
        samples = [i * 1000 for i in range(-7, 7)]
        dsp, bfstm = self._both(ref_bin, samples)
        assert ref_adpcm(dsp, 1) == bfstm_adpcm(bfstm, 8), \
            "ADPCM mismatch (single frame)"

    def test_adpcm_large(self, ref_bin):
        """Larger input: 100 frames — verifies history propagation across frames."""
        n = 14 * 100
        samples = [int(12000 * math.sin(2 * math.pi * i / 47)) for i in range(n)]
        dsp, bfstm = self._both(ref_bin, samples)
        frames = n // 14
        assert ref_adpcm(dsp, frames) == bfstm_adpcm(bfstm, frames * 8), \
            "ADPCM mismatch (100 frames)"
