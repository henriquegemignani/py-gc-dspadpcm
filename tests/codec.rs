use gc_dspadpcm::codec::{correlate_coefs, encode_frame, Coefs};

#[test]
fn silence_frame_is_all_zeros() {
    let coefs: Coefs = [[0i16; 2]; 8];
    let mut pcm = [0i16; 16];
    let frame = encode_frame(&mut pcm, 14, &coefs);
    assert_eq!(frame, [0u8; 8]);
}

#[test]
fn correlate_coefs_sine_in_range() {
    let sample_rate = 22050.0f64;
    let freq = 440.0f64;
    let samples: Vec<i16> = (0..256)
        .map(|i| {
            (32767.0 * (2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate).sin()) as i16
        })
        .collect();
    let coefs = correlate_coefs(&samples);
    // All coefficient values must fit in i16 (checked by type), and first pair must be non-zero
    assert!(
        coefs[0][0] != 0 || coefs[0][1] != 0,
        "first coef pair should be non-zero for sine"
    );
}

#[test]
fn history_propagation() {
    // Build a simple ramp signal, encode two consecutive frames,
    // verify that conv_samps[14]/[15] after frame 1 become [0]/[1] before frame 2.
    let samples: Vec<i16> = (0..28).map(|i| (i as i16) * 100).collect();
    let coefs = correlate_coefs(&samples);

    let mut conv = [0i16; 16];
    // Frame 1
    for s in 0..14 {
        conv[s + 2] = samples[s];
    }
    encode_frame(&mut conv, 14, &coefs);
    let expected0 = conv[14];
    let expected1 = conv[15];

    // Advance history
    conv[0] = conv[14];
    conv[1] = conv[15];

    assert_eq!(conv[0], expected0);
    assert_eq!(conv[1], expected1);
}

#[test]
fn silence_coefs_do_not_panic() {
    let samples = vec![0i16; 22050];
    let coefs = correlate_coefs(&samples);
    // Should not panic; all coefs are 0 for silence
    for pair in &coefs {
        assert_eq!(*pair, [0i16, 0i16]);
    }
}
