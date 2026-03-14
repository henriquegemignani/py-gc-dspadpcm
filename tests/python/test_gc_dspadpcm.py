import struct
import wave
import io
import math
import pytest
import gc_dspadpcm


def make_sine_wav(freq=440, duration=0.5, sample_rate=22050) -> bytes:
    """Return bytes of a minimal mono 16-bit PCM WAV."""
    samples = [
        int(32767 * math.sin(2 * math.pi * freq * i / sample_rate))
        for i in range(int(sample_rate * duration))
    ]
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sample_rate)
        w.writeframes(struct.pack(f"<{len(samples)}h", *samples))
    return buf.getvalue()


def test_encode_wav_returns_bytes():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    assert isinstance(bfstm, bytes)


def test_encode_wav_magic():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    assert bfstm[:4] == b"FSTM"


def test_encode_wav_bom():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    assert bfstm[4:6] == b"\xff\xfe"


def test_encode_wav_file_size_field():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    file_size = struct.unpack_from("<I", bfstm, 0x0C)[0]
    assert file_size == len(bfstm)


def test_encode_wav_section_count():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    assert struct.unpack_from("<H", bfstm, 0x10)[0] == 3


def test_encode_pcm_matches_encode_wav():
    wav = make_sine_wav()
    with wave.open(io.BytesIO(wav)) as w:
        pcm = w.readframes(w.getnframes())
        sr = w.getframerate()
    assert gc_dspadpcm.encode_wav(wav) == gc_dspadpcm.encode_pcm(pcm, sr)


def test_encode_wav_rejects_stereo():
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(22050)
        w.writeframes(b"\x00" * 88200)
    with pytest.raises(ValueError, match="mono"):
        gc_dspadpcm.encode_wav(buf.getvalue())


def test_encode_wav_rejects_8bit():
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(1)
        w.setframerate(22050)
        w.writeframes(b"\x80" * 22050)
    with pytest.raises(ValueError, match="16-bit"):
        gc_dspadpcm.encode_wav(buf.getvalue())


def test_encode_wav_silence():
    """Silence should produce a valid BFSTM without panicking."""
    buf = io.BytesIO()
    n = 22050
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(22050)
        w.writeframes(b"\x00" * (n * 2))
    bfstm = gc_dspadpcm.encode_wav(buf.getvalue())
    assert bfstm[:4] == b"FSTM"


def test_sections_are_0x20_aligned():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    # Each 12-byte section ref: u16 type, u16 pad, u32 offset, u32 size.
    # Offset fields at 0x18, 0x24, 0x30 (= 0x18 + i*0x0C).
    for i in range(3):
        off = struct.unpack_from("<I", bfstm, 0x18 + i * 0x0C)[0]
        assert off % 0x20 == 0, f"section {i} offset {off:#x} not 0x20-aligned"


def test_data_section_magic():
    bfstm = gc_dspadpcm.encode_wav(make_sine_wav())
    # DATA section offset is at 0x30 (third ref's offset field)
    data_off = struct.unpack_from("<I", bfstm, 0x30)[0]
    assert bfstm[data_off : data_off + 4] == b"DATA"
