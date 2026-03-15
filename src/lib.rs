pub mod codec;
pub mod bfstm;

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
    if pcm_data.len() % 2 != 0 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "pcm_data length must be a multiple of 2 (16-bit samples)",
        ));
    }
    let samples: Vec<i16> = pcm_data
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    let bfstm = bfstm::build_bfstm(&samples, sample_rate);
    Ok(PyBytes::new_bound(py, &bfstm).into())
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
    let (sample_rate, pcm_bytes) = parse_wav(wav_data)?;
    let samples: Vec<i16> = pcm_bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    let bfstm = bfstm::build_bfstm(&samples, sample_rate);
    Ok(PyBytes::new_bound(py, &bfstm).into())
}

/// Minimal WAV parser.  Returns (sample_rate, pcm_bytes_slice).
fn parse_wav(data: &[u8]) -> PyResult<(u32, &[u8])> {
    fn read_u16(d: &[u8], off: usize) -> PyResult<u16> {
        d.get(off..off + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("WAV data truncated"))
    }
    fn read_u32(d: &[u8], off: usize) -> PyResult<u32> {
        d.get(off..off + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("WAV data truncated"))
    }

    if data.len() < 12 {
        return Err(pyo3::exceptions::PyValueError::new_err("Not a valid WAV file"));
    }
    if &data[0..4] != b"RIFF" {
        return Err(pyo3::exceptions::PyValueError::new_err("Not a valid RIFF file"));
    }
    if &data[8..12] != b"WAVE" {
        return Err(pyo3::exceptions::PyValueError::new_err("Not a valid WAVE file"));
    }

    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut fmt_ok = false;

    while pos + 8 <= data.len() {
        let tag = &data[pos..pos + 4];
        let chunk_size = read_u32(data, pos + 4)? as usize;
        pos += 8;

        if tag == b"fmt " {
            if chunk_size < 16 {
                return Err(pyo3::exceptions::PyValueError::new_err("fmt chunk too small"));
            }
            let format = read_u16(data, pos)?;
            if format != 1 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Only PCM (format=1) WAV files are supported",
                ));
            }
            let channels = read_u16(data, pos + 2)?;
            if channels != 1 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Only mono WAV files are supported",
                ));
            }
            sample_rate = read_u32(data, pos + 4)?;
            let bits_per_sample = read_u16(data, pos + 14)?;
            if bits_per_sample != 16 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Only 16-bit WAV files are supported",
                ));
            }
            fmt_ok = true;
        } else if tag == b"data" {
            if !fmt_ok {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "WAV data chunk before fmt chunk",
                ));
            }
            let end = pos + chunk_size;
            if end > data.len() {
                return Err(pyo3::exceptions::PyValueError::new_err("WAV data truncated"));
            }
            return Ok((sample_rate, &data[pos..end]));
        }

        pos += chunk_size;
        // Chunks are word-aligned
        if chunk_size % 2 != 0 {
            pos += 1;
        }
    }

    Err(pyo3::exceptions::PyValueError::new_err(
        "WAV file missing data chunk",
    ))
}

#[pymodule]
fn _lib(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(encode_pcm, m)?)?;
    m.add_function(wrap_pyfunction!(encode_wav, m)?)?;
    Ok(())
}
