use crate::codec::{self, Coefs};

// ── Layout constants (Dread-compatible) ──────────────────────────────────────
const BLOCK_SIZE: u32 = 0x2000; // 8 192 bytes per block
const BLOCK_SAMPLE_COUNT: u32 = 0x3800; // 14 336 samples per block
const SEEK_INTERVAL: u32 = 0x3800; // one seek entry per block
const ADPCM_CODEC: u8 = 2;

// Reference-type constants used inside the INFO section
const RT_STREAM_INFO_BLOCK: u16 = 0x4000;
const RT_STREAM_SEEK_BLOCK: u16 = 0x4001;
const RT_STREAM_DATA_BLOCK: u16 = 0x4002;
const RT_STREAM_INFO: u16 = 0x4100;
const RT_REF_TABLE: u16 = 0x0101;
const RT_CHANNEL_INFO: u16 = 0x4102;
const RT_GC_ADPCM_INFO: u16 = 0x0300;
const RT_SAMPLE_DATA: u16 = 0x1F00;

// ── Small writer helpers ─────────────────────────────────────────────────────

fn push_u8(buf: &mut Vec<u8>, v: u8) {
    buf.push(v);
}
fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn push_i16(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn push_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn push_zeros(buf: &mut Vec<u8>, n: usize) {
    buf.resize(buf.len() + n, 0);
}
fn align_to(buf: &mut Vec<u8>, alignment: usize) {
    let rem = buf.len() % alignment;
    if rem != 0 {
        push_zeros(buf, alignment - rem);
    }
}
fn round_up(v: u32, align: u32) -> u32 {
    v.div_ceil(align) * align
}
fn patch_u32(buf: &mut [u8], offset: usize, v: u32) {
    buf[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
}

// ── Reference helpers ────────────────────────────────────────────────────────

// Write a 8-byte sized-section reference: type(u16) pad(u16) offset(u32) size(u32)
fn push_section_ref(buf: &mut Vec<u8>, rt: u16, offset: u32, size: u32) {
    push_u16(buf, rt);
    push_u16(buf, 0);
    push_u32(buf, offset);
    push_u32(buf, size);
}

// Write a 8-byte block reference: type(u16) pad(u16) offset(u32)
fn push_block_ref(buf: &mut Vec<u8>, rt: u16, offset: u32) {
    push_u16(buf, rt);
    push_u16(buf, 0);
    push_u32(buf, offset);
}

// Write a null reference (8 bytes)
fn push_null_ref(buf: &mut Vec<u8>) {
    push_u32(buf, 0);
    push_i32(buf, -1);
}

// ── INFO section builder ─────────────────────────────────────────────────────

struct InfoParams<'a> {
    sample_count: u32,
    sample_rate: u32,
    block_count: u32,
    last_block_sample_count: u32,
    last_block_size_raw: u32,
    last_block_padded_size: u32,
    coefs: &'a Coefs,
    pred_scale0: i16,
}

fn build_info_section(p: InfoParams<'_>) -> Vec<u8> {
    let InfoParams {
        sample_count,
        sample_rate,
        block_count,
        last_block_sample_count,
        last_block_size_raw,
        last_block_padded_size,
        coefs,
        pred_scale0,
    } = p;
    let mut sec = Vec::new();

    // Section header placeholder (magic + size filled later)
    sec.extend_from_slice(b"INFO");
    push_u32(&mut sec, 0); // size placeholder

    // ── 3-entry reference table (24 bytes, base = offset 0x08 from section start) ──
    // Offsets are relative to this table's start (sec[8..]).
    // StreamInfo: offset 24 (right after this 24-byte table)
    push_block_ref(&mut sec, RT_STREAM_INFO, 24);
    // TrackInfo: null
    push_null_ref(&mut sec);
    // ChannelInfo table: offset 24 (StreamInfo) + 56 (InfoBlock1) = 80
    push_block_ref(&mut sec, RT_REF_TABLE, 80);

    // ── InfoBlock1: StreamInfo (56 = 0x38 bytes) ────────────────────────────
    // Byte breakdown: 4 (codec…) + 4*11 (int fields) + 8 (sample_data ref) = 56
    push_u8(&mut sec, ADPCM_CODEC);
    push_u8(&mut sec, 0); // loop_flag
    push_u8(&mut sec, 1); // channel_count
    push_u8(&mut sec, 0); // region_count

    push_u32(&mut sec, sample_rate);
    push_u32(&mut sec, 0); // loop_start
    push_u32(&mut sec, sample_count);
    push_u32(&mut sec, block_count); // interleave_count
    push_u32(&mut sec, BLOCK_SIZE); // interleave_size
    push_u32(&mut sec, BLOCK_SAMPLE_COUNT); // samples_per_interleave
    push_u32(&mut sec, last_block_size_raw);
    push_u32(&mut sec, last_block_sample_count);
    push_u32(&mut sec, last_block_padded_size);
    push_u32(&mut sec, 4); // bytes_per_seek_entry
    push_u32(&mut sec, SEEK_INTERVAL); // samples_per_seek_entry

    // SampleData reference — offset 0x18 from DATA section body start
    push_block_ref(&mut sec, RT_SAMPLE_DATA, 0x18);

    // ── InfoBlock3: ChannelInfoTable (66 = 0x42 bytes) ──────────────────────
    //
    // Layout:
    //   +0   u32 channel_count = 1
    //   +4   ref[0]: (RT_CHANNEL_INFO, 0, 12)  → points to +12 in this block
    //   +12  ref: (RT_GC_ADPCM_INFO,  0, 8)    → points to +8 from this ref = +20
    //   +20  DspAdpcmInfo (46 bytes)
    //
    push_u32(&mut sec, 1); // channel count

    // Channel ref table entry: offset = channelTableSize = 4 + 8*1 = 12
    push_block_ref(&mut sec, RT_CHANNEL_INFO, 12);

    // GcAdpcmInfo ref: offset = channelTable2Size - 8*0 + 0x2e*0 = 8
    push_block_ref(&mut sec, RT_GC_ADPCM_INFO, 8);

    // DspAdpcmInfo (46 bytes)
    for pair in coefs.iter() {
        push_i16(&mut sec, pair[0]);
        push_i16(&mut sec, pair[1]);
    }
    // StartContext: pred_scale, hist1=0, hist2=0
    push_i16(&mut sec, pred_scale0);
    push_i16(&mut sec, 0);
    push_i16(&mut sec, 0);
    // LoopContext: all zero (not looping)
    push_i16(&mut sec, 0);
    push_i16(&mut sec, 0);
    push_i16(&mut sec, 0);
    // Padding
    push_u16(&mut sec, 0);

    // Pad to 0x20 and fill size
    align_to(&mut sec, 0x20);
    let section_size = sec.len() as u32;
    patch_u32(&mut sec, 4, section_size);
    sec
}

// ── Public function ──────────────────────────────────────────────────────────

/// Build a complete, valid BFSTM file from raw mono 16-bit PCM samples.
pub fn build_bfstm(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let sample_count = samples.len() as u32;
    let packet_count = sample_count.div_ceil(14) as usize;
    let block_count = sample_count.div_ceil(BLOCK_SAMPLE_COUNT).max(1);
    let frames_per_block = (BLOCK_SAMPLE_COUNT / 14) as usize; // = 1024

    let last_block_sample_count = if sample_count == 0 {
        0
    } else {
        let r = sample_count % BLOCK_SAMPLE_COUNT;
        if r == 0 {
            BLOCK_SAMPLE_COUNT
        } else {
            r
        }
    };
    let last_block_frames = last_block_sample_count.div_ceil(14);
    let last_block_size_raw = last_block_frames * 8;
    let last_block_padded_size = round_up(last_block_size_raw, 0x20);

    // ── Encode ────────────────────────────────────────────────────────────────
    let coefs = codec::correlate_coefs(samples);
    let mut adpcm_frames: Vec<u8> = Vec::with_capacity(packet_count * 8);
    let mut conv_samps = [0i16; 16];
    let mut block_end_hist1: Vec<i16> = Vec::new();

    for p in 0..packet_count {
        for s in 0..14usize {
            let idx = p * 14 + s;
            conv_samps[s + 2] = if idx < samples.len() { samples[idx] } else { 0 };
        }
        let frame = codec::encode_frame(&mut conv_samps, 14, &coefs);
        adpcm_frames.extend_from_slice(&frame);
        conv_samps[0] = conv_samps[14];
        conv_samps[1] = conv_samps[15];

        // Record hist1 at end of each full block
        if (p + 1) % frames_per_block == 0 {
            block_end_hist1.push(conv_samps[1]);
        }
    }

    let pred_scale0 = if adpcm_frames.is_empty() {
        0
    } else {
        adpcm_frames[0] as i16
    };

    // ── Seek entries ──────────────────────────────────────────────────────────
    // entry[0] = all zeros (state before block 0)
    // entry[i] = (pred_scale of block i's first frame, hist1 at end of block i-1)
    let mut seek_entries: Vec<(i16, i16)> = vec![(0, 0)];
    for i in 1..(block_count as usize) {
        let pred_scale = adpcm_frames
            .get(i * frames_per_block * 8)
            .copied()
            .unwrap_or(0) as i16;
        let hist1 = block_end_hist1.get(i - 1).copied().unwrap_or(0);
        seek_entries.push((pred_scale, hist1));
    }

    // ── Section sizes ─────────────────────────────────────────────────────────
    let header_size: u32 = 0x40;

    let info_section = build_info_section(InfoParams {
        sample_count,
        sample_rate,
        block_count,
        last_block_sample_count,
        last_block_size_raw,
        last_block_padded_size,
        coefs: &coefs,
        pred_scale0,
    });
    let info_size = info_section.len() as u32;

    let seek_payload_bytes = (seek_entries.len() as u32) * 4; // 4 bytes per entry (mono)
    let seek_size = round_up(8 + seek_payload_bytes, 0x20);

    let audio_data_size = round_up(adpcm_frames.len() as u32, 0x20);
    let data_size: u32 = 0x20 + audio_data_size;

    let file_size = header_size + info_size + seek_size + data_size;

    let info_offset = header_size;
    let seek_offset = info_offset + info_size;
    let data_offset = seek_offset + seek_size;

    // ── Assemble file ─────────────────────────────────────────────────────────
    let mut file: Vec<u8> = Vec::with_capacity(file_size as usize);

    // ── File header (0x40 bytes) ──────────────────────────────────────────────
    file.extend_from_slice(b"FSTM");
    push_u16(&mut file, 0xFEFF); // BOM (little-endian)
    push_u16(&mut file, 0x0040); // header size
    push_u32(&mut file, 0x00040006); // version
    push_u32(&mut file, file_size);
    push_u16(&mut file, 3); // section count
    push_u16(&mut file, 0); // padding

    push_section_ref(&mut file, RT_STREAM_INFO_BLOCK, info_offset, info_size);
    push_section_ref(&mut file, RT_STREAM_SEEK_BLOCK, seek_offset, seek_size);
    push_section_ref(&mut file, RT_STREAM_DATA_BLOCK, data_offset, data_size);

    // Pad header to 0x40
    let header_pad = (header_size as usize).saturating_sub(file.len());
    push_zeros(&mut file, header_pad);

    // ── INFO section ──────────────────────────────────────────────────────────
    file.extend_from_slice(&info_section);

    // ── SEEK section ──────────────────────────────────────────────────────────
    assert_eq!(file.len() as u32, seek_offset);
    file.extend_from_slice(b"SEEK");
    push_u32(&mut file, seek_size);
    for (pred_scale, hist1) in &seek_entries {
        push_i16(&mut file, *pred_scale);
        push_i16(&mut file, *hist1);
    }
    // Pad to seek_size
    while file.len() < (seek_offset + seek_size) as usize {
        file.push(0);
    }

    // ── DATA section ──────────────────────────────────────────────────────────
    assert_eq!(file.len() as u32, data_offset);
    file.extend_from_slice(b"DATA");
    push_u32(&mut file, data_size);
    // Padding to +0x20 from DATA section start
    push_zeros(&mut file, 0x18); // 8 (magic+size) + 0x18 = 0x20

    file.extend_from_slice(&adpcm_frames);
    // Trailing zero-pad for audio_data_size alignment
    while file.len() < (data_offset + data_size) as usize {
        file.push(0);
    }

    file
}
