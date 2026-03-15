use gc_dspadpcm::bfstm::build_bfstm;

fn read_u16_le(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}
fn read_u32_le(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn make_sine(freq: f64, duration: f64, sample_rate: u32) -> Vec<i16> {
    let n = (sample_rate as f64 * duration) as usize;
    (0..n)
        .map(|i| {
            (32767.0 * (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate as f64).sin())
                as i16
        })
        .collect()
}

#[test]
fn magic_and_bom() {
    let samples = make_sine(440.0, 0.1, 22050);
    let bfstm = build_bfstm(&samples, 22050);
    assert_eq!(&bfstm[0..4], b"FSTM");
    assert_eq!(&bfstm[4..6], &[0xFF, 0xFE]);
}

#[test]
fn file_size_field_matches_actual_length() {
    let samples = make_sine(440.0, 0.5, 22050);
    let bfstm = build_bfstm(&samples, 22050);
    let field = read_u32_le(&bfstm, 0x0C);
    assert_eq!(field as usize, bfstm.len());
}

#[test]
fn section_count_is_three() {
    let samples = make_sine(440.0, 0.1, 22050);
    let bfstm = build_bfstm(&samples, 22050);
    assert_eq!(read_u16_le(&bfstm, 0x10), 3);
}

#[test]
fn sections_are_0x20_aligned() {
    let samples = make_sine(440.0, 0.5, 22050);
    let bfstm = build_bfstm(&samples, 22050);
    // Each 12-byte section ref: u16 type, u16 pad, u32 offset, u32 size.
    // Offsets: INFO=0x18, SEEK=0x24, DATA=0x30
    for slot in 0..3usize {
        let offset = read_u32_le(&bfstm, 0x18 + slot * 0x0C) as usize;
        assert_eq!(
            offset % 0x20,
            0,
            "section {slot} offset {offset:#x} not aligned"
        );
    }
}

#[test]
fn data_section_magic_and_payload_size() {
    let samples = make_sine(440.0, 0.25, 22050);
    let n = samples.len();
    let bfstm = build_bfstm(&samples, 22050);

    // DATA section offset is at 0x30 (third 12-byte ref's offset field: 0x2C+4=0x30)
    let data_off = read_u32_le(&bfstm, 0x30) as usize;
    assert_eq!(&bfstm[data_off..data_off + 4], b"DATA");

    // Payload (raw ADPCM frames): one 8-byte frame per 14 samples
    let expected_frame_bytes = ((n + 13) / 14) * 8;
    let audio_start = data_off + 0x20;
    let data_size = read_u32_le(&bfstm, data_off + 4) as usize;
    let audio_region_size = data_size - 0x20;
    // audio_region_size may be padded to 0x20, so frames must fit within it
    assert!(expected_frame_bytes <= audio_region_size);
    assert_eq!(audio_region_size % 0x20, 0);
    // The unpadded frame bytes sit at the start
    assert!(audio_start + expected_frame_bytes <= bfstm.len());
}

#[test]
fn silence_builds_without_panic() {
    let samples = vec![0i16; 22050];
    let bfstm = build_bfstm(&samples, 22050);
    assert_eq!(&bfstm[0..4], b"FSTM");
}

#[test]
fn multi_block_has_correct_seek_entry_count() {
    // 3 blocks worth of samples
    let n = 3 * 0x3800usize;
    let samples: Vec<i16> = (0..n).map(|i| ((i % 256) as i16) * 100).collect();
    let bfstm = build_bfstm(&samples, 22050);

    // SEEK section offset is at 0x24 (second 12-byte ref's offset field: 0x20+4=0x24)
    let seek_off = read_u32_le(&bfstm, 0x24) as usize;
    assert_eq!(&bfstm[seek_off..seek_off + 4], b"SEEK");
    let seek_size = read_u32_le(&bfstm, seek_off + 4) as usize;
    // 3 blocks → 3 seek entries × 4 bytes = 12 bytes payload + 8 header = 20 → padded to 32
    let expected_payload = 3 * 4;
    let expected_size = (8 + expected_payload + 0x1F) & !0x1F;
    assert_eq!(seek_size, expected_size);
}
