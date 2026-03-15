//! Direct port of grok.c (DSP-ADPCM codec by jackoalan, MIT licence).
//! All integer arithmetic and floating-point operations match the original.
#![allow(clippy::needless_range_loop)]

type TVec = [f64; 3];
type CoefPair = [i16; 2];
pub type Coefs = [CoefPair; 8];

// ── Internal helpers ────────────────────────────────────────────────────────
//
// The C code calls these with pcmHistBuffer[1] (14 samples) where negative
// indices reach into pcmHistBuffer[0].  We represent both halves as a flat
// [i16; 28] buffer and pass `offset = 14` so that buf[offset + x - i] maps
// to the correct sample.

fn inner_product_merge(vec_out: &mut TVec, buf: &[i16; 28], offset: usize) {
    for i in 0..=2usize {
        vec_out[i] = 0.0;
        for x in 0..14usize {
            vec_out[i] -= buf[offset + x - i] as f64 * buf[offset + x] as f64;
        }
    }
}

fn outer_product_merge(mtx_out: &mut [TVec; 3], buf: &[i16; 28], offset: usize) {
    for x in 1..=2usize {
        for y in 1..=2usize {
            mtx_out[x][y] = 0.0;
            for z in 0..14usize {
                mtx_out[x][y] += buf[offset + z - x] as f64 * buf[offset + z - y] as f64;
            }
        }
    }
}

fn analyze_ranges(mtx: &mut [TVec; 3], vec_idxs_out: &mut [usize; 3]) -> bool {
    let mut recips = [0.0f64; 3];

    for x in 1..=2usize {
        let val = mtx[x][1].abs().max(mtx[x][2].abs());
        if val < f64::EPSILON {
            return true;
        }
        recips[x] = 1.0 / val;
    }

    let mut max_index = 0usize;
    for i in 1..=2usize {
        for x in 1..i {
            let mut tmp = mtx[x][i];
            for y in 1..x {
                tmp -= mtx[x][y] * mtx[y][i];
            }
            mtx[x][i] = tmp;
        }

        let mut val = 0.0f64;
        for x in i..=2usize {
            let mut tmp = mtx[x][i];
            for y in 1..i {
                tmp -= mtx[x][y] * mtx[y][i];
            }
            mtx[x][i] = tmp;
            let t = tmp.abs() * recips[x];
            if t >= val {
                val = t;
                max_index = x;
            }
        }

        if max_index != i {
            for y in 1..=2usize {
                let tmp = mtx[max_index][y];
                mtx[max_index][y] = mtx[i][y];
                mtx[i][y] = tmp;
            }
            recips[max_index] = recips[i];
        }

        vec_idxs_out[i] = max_index;

        if mtx[i][i] == 0.0 {
            return true;
        }

        if i != 2 {
            let tmp = 1.0 / mtx[i][i];
            for x in (i + 1)..=2usize {
                mtx[x][i] *= tmp;
            }
        }
    }

    let mut min = 1.0e10f64;
    let mut max = 0.0f64;
    for i in 1..=2usize {
        let tmp = mtx[i][i].abs();
        if tmp < min {
            min = tmp;
        }
        if tmp > max {
            max = tmp;
        }
    }

    if min / max < 1.0e-10 {
        return true;
    }

    false
}

fn bidirectional_filter(mtx: &mut [TVec; 3], vec_idxs: &[usize; 3], vec_out: &mut TVec) {
    let mut x = 0usize;
    for i in 1..=2usize {
        let index = vec_idxs[i];
        let mut tmp = vec_out[index];
        vec_out[index] = vec_out[i];
        if x != 0 {
            for y in x..i {
                tmp -= vec_out[y] * mtx[i][y];
            }
        } else if tmp != 0.0 {
            x = i;
        }
        vec_out[i] = tmp;
    }

    for i in (1..=2usize).rev() {
        let mut tmp = vec_out[i];
        for y in (i + 1)..=2usize {
            tmp -= vec_out[y] * mtx[i][y];
        }
        vec_out[i] = tmp / mtx[i][i];
    }

    vec_out[0] = 1.0;
}

fn quadratic_merge(in_out_vec: &mut TVec) -> bool {
    let v2 = in_out_vec[2];
    let tmp = 1.0 - v2 * v2;
    if tmp == 0.0 {
        return true;
    }
    let v0 = (in_out_vec[0] - v2 * v2) / tmp;
    let v1 = (in_out_vec[1] - in_out_vec[1] * v2) / tmp;
    in_out_vec[0] = v0;
    in_out_vec[1] = v1;
    v1.abs() > 1.0
}

fn finish_record(in_vec: &mut TVec, out: &mut TVec) {
    for z in 1..=2usize {
        if in_vec[z] >= 1.0 {
            in_vec[z] = 0.9999999999;
        } else if in_vec[z] <= -1.0 {
            in_vec[z] = -0.9999999999;
        }
    }
    out[0] = 1.0;
    out[1] = (in_vec[2] * in_vec[1]) + in_vec[1];
    out[2] = in_vec[2];
}

fn matrix_filter(src: &TVec, dst: &mut TVec) {
    let mut mtx: [TVec; 3] = [[0.0; 3]; 3];
    mtx[2][0] = 1.0;
    for i in 1..=2usize {
        mtx[2][i] = -src[i];
    }
    for i in (1..=2usize).rev() {
        let val = 1.0 - mtx[i][i] * mtx[i][i];
        for y in 1..=i {
            mtx[i - 1][y] = ((mtx[i][i] * mtx[i][y]) + mtx[i][y]) / val;
        }
    }
    dst[0] = 1.0;
    for i in 1..=2usize {
        dst[i] = 0.0;
        for y in 1..=i {
            dst[i] += mtx[i][y] * dst[i - y];
        }
    }
}

fn merge_finish_record(src: &TVec, dst: &mut TVec) {
    let mut tmp: TVec = [0.0; 3];
    let mut val = src[0];
    dst[0] = 1.0;
    for i in 1..=2usize {
        let mut v2 = 0.0f64;
        for y in 1..i {
            v2 += dst[y] * src[i - y];
        }
        dst[i] = if val > 0.0 { -(v2 + src[i]) / val } else { 0.0 };
        tmp[i] = dst[i];
        for y in 1..i {
            dst[y] += dst[i] * dst[i - y];
        }
        val *= 1.0 - dst[i] * dst[i];
    }
    finish_record(&mut tmp, dst);
}

fn contrast_vectors(source1: &TVec, source2: &TVec) -> f64 {
    let val = (source2[2] * source2[1] + -source2[1]) / (1.0 - source2[2] * source2[2]);
    let val1 = source1[0] * source1[0] + source1[1] * source1[1] + source1[2] * source1[2];
    let val2 = source1[0] * source1[1] + source1[1] * source1[2];
    let val3 = source1[0] * source1[2];
    val1 + 2.0 * val * val2 + 2.0 * (-source2[1] * val + -source2[2]) * val3
}

fn filter_records(vec_best: &mut [TVec; 8], exp: usize, records: &[TVec]) {
    let record_count = records.len();
    let mut buffer_list: [TVec; 8] = [[0.0; 3]; 8];
    let mut buffer1: [i32; 8] = [0; 8];
    let mut buffer2: TVec = [0.0; 3];

    for _x in 0..2 {
        for y in 0..exp {
            buffer1[y] = 0;
            buffer_list[y] = [0.0; 3];
        }
        for z in 0..record_count {
            let mut index = 0usize;
            let mut value = 1.0e30f64;
            for i in 0..exp {
                let temp_val = contrast_vectors(&vec_best[i], &records[z]);
                if temp_val < value {
                    value = temp_val;
                    index = i;
                }
            }
            buffer1[index] += 1;
            matrix_filter(&records[z], &mut buffer2);
            for i in 0..=2usize {
                buffer_list[index][i] += buffer2[i];
            }
        }
        for i in 0..exp {
            if buffer1[i] > 0 {
                let n = buffer1[i] as f64;
                for y in 0..=2usize {
                    buffer_list[i][y] /= n;
                }
            }
        }
        for i in 0..exp {
            let src = buffer_list[i];
            merge_finish_record(&src, &mut vec_best[i]);
        }
    }
}

// ── Public functions ─────────────────────────────────────────────────────────

/// Analyse `samples` (mono 16-bit PCM) and return 8 DSP-ADPCM coefficient pairs.
/// Direct port of DSPCorrelateCoefs from grok.c.
pub fn correlate_coefs(samples: &[i16]) -> Coefs {
    if samples.is_empty() {
        return [[0i16; 2]; 8];
    }

    let num_frames = samples.len().div_ceil(14);
    let mut records: Vec<TVec> = Vec::with_capacity(num_frames * 2);

    // Flat [prev 14 | curr 14] history buffer, mirroring pcmHistBuffer[2][14].
    let mut pcm_hist: [i16; 28] = [0i16; 28];

    let mut vec1: TVec = [0.0; 3];
    let mut mtx: [TVec; 3] = [[0.0; 3]; 3];
    let mut vec_idxs: [usize; 3] = [0; 3];

    let mut src_pos = 0usize;
    let total = samples.len();

    while src_pos < total {
        let frame_samples = (total - src_pos).min(0x3800);
        let block_start = src_pos;
        src_pos += frame_samples;

        let mut i = 0usize;
        while i < frame_samples {
            // Shift: prev ← curr (use split to satisfy borrow checker)
            let (prev, curr) = pcm_hist.split_at_mut(14);
            prev.copy_from_slice(curr);

            // Fill current 14-sample window (zero-pad at end of last block)
            for z in 0..14usize {
                let idx = i + z;
                pcm_hist[14 + z] = if idx < frame_samples {
                    samples[block_start + idx]
                } else {
                    0
                };
            }
            i += 14;

            inner_product_merge(&mut vec1, &pcm_hist, 14);
            if vec1[0].abs() > 10.0 {
                outer_product_merge(&mut mtx, &pcm_hist, 14);
                if !analyze_ranges(&mut mtx, &mut vec_idxs) {
                    bidirectional_filter(&mut mtx, &vec_idxs, &mut vec1);
                    if !quadratic_merge(&mut vec1) {
                        let mut record: TVec = [0.0; 3];
                        finish_record(&mut vec1, &mut record);
                        records.push(record);
                    }
                }
            }
        }
    }

    // Degenerate case: silence or very short input
    if records.is_empty() {
        return [[0i16; 2]; 8];
    }

    let record_count = records.len();
    vec1 = [1.0, 0.0, 0.0];

    let mut vec_best: [TVec; 8] = [[0.0; 3]; 8];
    let mut tmp_best: TVec = [0.0; 3];
    for z in 0..record_count {
        matrix_filter(&records[z], &mut tmp_best);
        for y in 1..=2usize {
            vec1[y] += tmp_best[y];
        }
    }
    for y in 1..=2usize {
        vec1[y] /= record_count as f64;
    }
    merge_finish_record(&vec1, &mut vec_best[0]);

    let mut exp = 1usize;
    for w in 0..3 {
        let vec2: TVec = [0.0, -1.0, 0.0];
        for i in 0..exp {
            for y in 0..=2usize {
                vec_best[exp + i][y] = 0.01 * vec2[y] + vec_best[i][y];
            }
        }
        let _ = w;
        exp = 1 << (w + 1);
        filter_records(&mut vec_best, exp, &records);
    }

    // Convert to i16 coefficient pairs
    let mut coefs: Coefs = [[0i16; 2]; 8];
    for z in 0..8usize {
        let d0 = -vec_best[z][1] * 2048.0;
        coefs[z][0] = if d0 > 32767.0 {
            32767
        } else if d0 < -32768.0 {
            -32768
        } else {
            d0.round() as i16
        };

        let d1 = -vec_best[z][2] * 2048.0;
        coefs[z][1] = if d1 > 32767.0 {
            32767
        } else if d1 < -32768.0 {
            -32768
        } else {
            d1.round() as i16
        };
    }
    coefs
}

/// Encode one DSP-ADPCM frame.
/// `pcm_inout[0..2]`  = history (yn2, yn1); `pcm_inout[2..16]` = 14 input samples.
/// After the call, `pcm_inout[2..16]` holds the reconstructed samples (use
/// `pcm_inout[14]` and `pcm_inout[15]` as the next frame's history).
/// Direct port of DSPEncodeFrame from grok.c.
pub fn encode_frame(pcm_inout: &mut [i16; 16], sample_count: usize, coefs: &Coefs) -> [u8; 8] {
    let mut in_samples = [[0i32; 16]; 8];
    let mut out_samples = [[0i32; 14]; 8];
    let mut best_index = 0usize;
    let mut scale = [0i32; 8];
    let mut dist_accum = [0.0f64; 8];

    for i in 0..8usize {
        in_samples[i][0] = pcm_inout[0] as i32;
        in_samples[i][1] = pcm_inout[1] as i32;

        let mut distance = 0i32;
        for s in 0..sample_count {
            let v1 = (pcm_inout[s] as i32 * coefs[i][1] as i32
                + pcm_inout[s + 1] as i32 * coefs[i][0] as i32)
                / 2048;
            in_samples[i][s + 2] = v1;
            let v2 = pcm_inout[s + 2] as i32 - v1;
            let v3 = v2.clamp(-32768, 32767);
            if v3.abs() > distance.abs() {
                distance = v3;
            }
        }

        // Initial scale estimation
        let mut temp_scale = 0i32;
        let mut temp_dist = distance;
        while temp_scale <= 12 && !(-8..=7).contains(&temp_dist) {
            temp_scale += 1;
            temp_dist /= 2;
        }
        scale[i] = if temp_scale <= 1 { -1 } else { temp_scale - 2 };

        loop {
            scale[i] += 1;
            dist_accum[i] = 0.0;
            let mut index = 0i32;

            for s in 0..sample_count {
                let v1 = in_samples[i][s] * coefs[i][1] as i32
                    + in_samples[i][s + 1] * coefs[i][0] as i32;
                // v2: residual divided by 2048 (matches C: ((pcm<<11)-v1)/2048)
                let v2 = (((pcm_inout[s + 2] as i32) << 11) - v1) / 2048;
                let v2_div = v2 as f64 / (1i32 << scale[i]) as f64;
                let v3_raw = if v2 > 0 {
                    (v2_div + 0.4999999) as i32
                } else {
                    (v2_div - 0.4999999) as i32
                };

                let v3 = if v3_raw < -8 {
                    let overflow = -8 - v3_raw;
                    if index < overflow {
                        index = overflow;
                    }
                    -8i32
                } else if v3_raw > 7 {
                    let overflow = v3_raw - 7;
                    if index < overflow {
                        index = overflow;
                    }
                    7i32
                } else {
                    v3_raw
                };

                out_samples[i][s] = v3;

                let v1_new = (v1 + ((v3 * (1 << scale[i])) << 11) + 1024) >> 11;
                let v2_clamped = v1_new.clamp(-32768, 32767);
                in_samples[i][s + 2] = v2_clamped;
                let err = pcm_inout[s + 2] as i32 - v2_clamped;
                dist_accum[i] += err as f64 * err as f64;
            }

            let mut x = index + 8;
            while x > 256 {
                scale[i] += 1;
                if scale[i] >= 12 {
                    scale[i] = 11;
                }
                x >>= 1;
            }

            if !(scale[i] < 12 && index > 1) {
                break;
            }
        }
    }

    let mut min = f64::MAX;
    for i in 0..8 {
        if dist_accum[i] < min {
            min = dist_accum[i];
            best_index = i;
        }
    }

    // Write reconstructed samples back
    for s in 0..sample_count {
        pcm_inout[s + 2] = in_samples[best_index][s + 2] as i16;
    }

    let mut adpcm_out = [0u8; 8];
    adpcm_out[0] = ((best_index << 4) | (scale[best_index] as usize & 0xF)) as u8;

    // Zero remaining out_samples slots
    for s in sample_count..14 {
        out_samples[best_index][s] = 0;
    }

    for y in 0..7usize {
        adpcm_out[y + 1] = ((out_samples[best_index][y * 2] << 4)
            | (out_samples[best_index][y * 2 + 1] & 0xF)) as u8;
    }

    adpcm_out
}

/// Encode all samples, returning coefficients and the raw ADPCM byte stream.
pub fn encode_all(samples: &[i16]) -> (Coefs, Vec<u8>) {
    let coefs = correlate_coefs(samples);
    let mut adpcm_frames: Vec<u8> = Vec::new();
    let mut conv_samps = [0i16; 16];
    let packet_count = samples.len().div_ceil(14);

    for p in 0..packet_count {
        let num_samples = (samples.len() - p * 14).min(14);
        for s in 0..num_samples {
            conv_samps[s + 2] = samples[p * 14 + s];
        }
        for s in num_samples..14 {
            conv_samps[s + 2] = 0;
        }
        let frame = encode_frame(&mut conv_samps, 14, &coefs);
        adpcm_frames.extend_from_slice(&frame);
        conv_samps[0] = conv_samps[14];
        conv_samps[1] = conv_samps[15];
    }
    (coefs, adpcm_frames)
}
