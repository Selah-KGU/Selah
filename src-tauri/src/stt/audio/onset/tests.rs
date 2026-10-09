use super::*;

#[test]
fn quiet_onset_is_replayed_once_before_the_audible_chunk() {
    let mut onset = OnsetBuffer::default();
    let quiet = vec![0.0004; 1280];
    assert!(onset.accept(quiet.clone(), false).is_none());
    let audible = vec![0.02; 1280];
    let joined = onset.accept(audible.clone(), true).unwrap();
    assert_eq!(joined.len(), quiet.len() + audible.len());
    assert_eq!(&joined[..quiet.len()], quiet);
    assert_eq!(&joined[quiet.len()..], audible);
    assert_eq!(onset.accept(audible.clone(), true).unwrap(), audible);
}

#[test]
fn long_room_tone_keeps_only_a_bounded_recent_prefix() {
    let mut onset = OnsetBuffer::default();
    for _ in 0..1000 {
        assert!(onset.accept(vec![0.0001; 1280], false).is_none());
        assert!(onset.retained.len() <= PRE_ROLL_SAMPLES);
        assert!(onset.retained.capacity() <= PRE_ROLL_SAMPLES);
    }
    let final_quiet: Vec<_> = (0..20_000).map(|i| i as f32 * 1e-8).collect();
    assert!(onset.accept(final_quiet.clone(), false).is_none());
    let joined = onset.accept(vec![0.2; 3], true).unwrap();
    assert_eq!(joined.len(), PRE_ROLL_SAMPLES + 3);
    assert_eq!(
        &joined[..PRE_ROLL_SAMPLES],
        &final_quiet[20_000 - PRE_ROLL_SAMPLES..]
    );
    assert_eq!(&joined[PRE_ROLL_SAMPLES..], &[0.2; 3]);
}

#[test]
fn microphone_callback_partitions_preserve_the_exact_recent_audio_order() {
    let quiet: Vec<_> = (0..10_000).map(|i| i as f32 * 1e-8).collect();
    for partition in [1, 7, 511, 1280, 16_000] {
        let mut onset = OnsetBuffer::default();
        for chunk in quiet.chunks(partition) {
            assert!(onset.accept(chunk.to_vec(), false).is_none());
        }
        let mut expected = quiet[quiet.len() - PRE_ROLL_SAMPLES..].to_vec();
        expected.extend([0.1, 0.2, 0.3]);
        let joined = onset.accept(vec![0.1, 0.2, 0.3], true).unwrap();
        assert_eq!(joined.len(), expected.len());
        assert_eq!(joined, expected);
    }
}

#[test]
fn empty_callbacks_do_not_release_audio_and_new_recordings_do_not_inherit_it() {
    let mut onset = OnsetBuffer::default();
    assert!(onset.accept(vec![0.0004; 10], false).is_none());
    assert!(onset.accept(Vec::new(), true).is_none());
    assert!(onset.accept(Vec::new(), false).is_none());
    let joined = onset.accept(vec![0.01; 5], true).unwrap();
    assert_eq!(joined.len(), 15);
    drop(onset);
    let mut next = OnsetBuffer::default();
    let incoming = vec![0.02; 100];
    let original = incoming.as_ptr();
    let untouched = next.accept(incoming, true).unwrap();
    assert_eq!(
        untouched.as_ptr(),
        original,
        "audible input unnecessarily copied"
    );
    assert_eq!(untouched, vec![0.02; 100]);
    assert_eq!(next.retained.capacity(), 0);
}
