use super::*;
use filmcraft_project::{Transcript, Word};
use filmcraft_speech::{Options, ProgressFn, SpeechError, Transcriber};

fn sec(n: f64) -> Tick {
    Tick::from_seconds_f64(n)
}
struct Fixed {
    candidates: Vec<GmCutCandidate>,
    fail: bool,
}
impl Transcriber for Fixed {
    fn id(&self) -> String {
        "fixed-gmcut".into()
    }
    fn transcribe(&self, _: &[f32], _: &Options, _: ProgressFn) -> std::result::Result<Transcript, SpeechError> {
        Ok(Transcript::default())
    }
    fn gmcut_candidates(&self, _: &[f32], _: f64, _: &str, _: FrameRate, _: ProgressFn) -> std::result::Result<Vec<GmCutCandidate>, SpeechError> {
        if self.fail { Err(SpeechError::Io("synthetic analysis failure".into())) } else { Ok(self.candidates.clone()) }
    }
}
fn values() -> Vec<GmCutCandidate> {
    vec![
        GmCutCandidate { start: sec(0.78), end: sec(1.96), pause_start: sec(0.7), pause_end: sec(2.), whole_only: false },
        GmCutCandidate { start: sec(2.48), end: sec(3.76), pause_start: sec(2.4), pause_end: sec(3.8), whole_only: false },
        GmCutCandidate { start: sec(4.5), end: sec(4.59), pause_start: sec(4.5), pause_end: sec(4.59), whole_only: true },
    ]
}
fn session(with_words: bool) -> (Session, ItemId) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.edit_sequence("Isolate synthetic GM컷", |q, _, _| {
        let duration = q.settings.frame_rate.tick_of(120);
        for (index, track) in q.audio_tracks.iter_mut().enumerate() {
            track.items.truncate(usize::from(index == 0));
            track.transitions.clear();
            for c in &mut track.items {
                c.start = Tick::ZERO;
                c.source_in = Tick::ZERO;
                c.duration = duration;
            }
        }
        for (index, track) in q.video_tracks.iter_mut().enumerate() {
            track.items.truncate(usize::from(index == 0));
            track.transitions.clear();
            for c in &mut track.items {
                c.start = Tick::ZERO;
                c.source_in = Tick::ZERO;
                c.duration = duration;
            }
        }
        q.caption_tracks.clear();
        q.markers.truncate(1);
        if let Some(m) = q.markers.first_mut() {
            m.start = sec(4.8);
        }
        Ok(())
    })
    .unwrap();
    let item = s.active_sequence().unwrap().audio_tracks[0].items[0].item;
    if with_words {
        let mut t = Transcript::default();
        for (text, a, b) in [("처음.", 0.2, 0.6), ("다음.", 2.1, 2.3), ("마지막.", 4., 4.3)] {
            t.words.push(Word::new(text, sec(a), sec(b)));
        }
        s.execute("transcript.set", json!({"item":item.0,"transcript":t})).unwrap();
        for name in ["Original subtitles", "Second subtitles"] {
            s.execute("transcript.createCaptions", json!({"name":name})).unwrap();
        }
    }
    s.transcriber = Some(Arc::new(Fixed { candidates: values(), fail: false }));
    (s, item)
}
fn analyze(s: &mut Session) -> Value {
    s.execute("gmcut.analyze", json!({})).unwrap();
    for _ in 0..1000 {
        let report = s.execute("gmcut.inspect", json!({})).unwrap();
        if report["running"] == false {
            return report;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("synthetic analysis timed out");
}
#[test]
fn gmcut_analyzer_does_not_change_existing_speech_availability() {
    let (mut s, _) = session(false);
    let gmcut_analyzer = s.transcriber.take();
    let speech_before = crate::transcript::can_transcribe(&s);
    s.gmcut_analyzer = gmcut_analyzer;
    assert_eq!(analyze(&mut s)["count"], 3);
    assert!(s.transcriber.is_none());
    assert_eq!(crate::transcript::can_transcribe(&s), speech_before);
}

#[test]
fn async_review_and_video_preview_are_read_only_and_undo_restores_cursor() {
    let (mut s, _) = session(false);
    let before = s.project.clone();
    let rev = s.revision;
    let undo = s.history.undo.len();
    let report = analyze(&mut s);
    assert_eq!(report["fresh"], true, "{report}");
    assert_eq!(report["count"], 3);
    assert_eq!(report["minSeconds"], 0.05);
    assert_eq!(report["keepSeconds"], 0.);
    s.execute("gmcut.choose", json!({"preset":1})).unwrap();
    assert_eq!(s.execute("gmcut.inspect", json!({})).unwrap()["cursor"], 1);
    s.execute("gmcut.skip", json!({})).unwrap();
    s.execute("gmcut.undo", json!({})).unwrap();
    s.execute("gmcut.undo", json!({})).unwrap();
    let report = s.execute("gmcut.inspect", json!({})).unwrap();
    assert_eq!(report["cursor"], 0);
    assert!(report["candidates"][0]["keepSeconds"].is_null());
    assert_eq!(s.playhead(), s.sequence_rate().snap(sec(0.7)));
    let preview = s.execute("gmcut.preview", json!({"index":0})).unwrap();
    assert!(preview["frames"].as_u64().unwrap() > 0);
    assert_eq!(preview["leftEnd"], preview["start"]);
    assert_eq!(preview["rightStart"], preview["end"]);
    assert!(s.gmcut.as_ref().unwrap().preview.is_some());
    assert_eq!(s.gmcut.as_ref().unwrap().preview_windows.len(), 2);
    assert!(Arc::ptr_eq(&before, &s.project));
    assert_eq!(s.revision, rev);
    assert_eq!(s.history.undo.len(), undo);
}
#[test]
fn raw_zero_removal_whole_only_and_positive_retention_match_standalone() {
    let (mut s, _) = session(false);
    analyze(&mut s);
    let rate = s.sequence_rate();
    let a = s.gmcut.as_ref().unwrap();
    let c = &a.candidates[0];
    let raw = cut(c, Tick::ZERO, rate).unwrap();
    let kept = cut(c, sec(0.2), rate).unwrap();
    assert!(raw.start < c.start, "zero removes raw quiet frames, without the 80ms guard");
    assert!(raw.end() > c.end);
    assert!(((c.pause_end - c.pause_start - kept.duration) - sec(0.2)).abs() <= rate.frame_duration());
    let whole = &a.candidates[2];
    assert!(cut(whole, Tick::ZERO, rate).is_some());
    assert!(cut(whole, sec(0.01), rate).is_none());
    s.execute("gmcut.auto", json!({"keepSeconds":0.2})).unwrap();
    let report = s.execute("gmcut.inspect", json!({})).unwrap();
    assert!(report["candidates"][2]["cut"].is_null());
}
#[test]
fn apply_preserves_sources_two_caption_tracks_and_markers_in_one_undo() {
    let (mut s, item) = session(true);
    s.state.ripple_sequence_markers = true;
    let before = s.project.clone();
    let undo = s.history.undo.len();
    let words = crate::transcript::sequence_words(&s).into_iter().map(|w| w.text).collect::<Vec<_>>();
    analyze(&mut s);
    s.execute("gmcut.auto", json!({})).unwrap();
    let result = s.execute("gmcut.apply", json!({})).unwrap();
    assert_eq!(result["removed"], 3);
    let total = Tick(result["ticks"].as_i64().unwrap());
    let q = s.active_sequence().unwrap();
    let old = before.sequence(s.state.active_sequence.unwrap()).unwrap();
    assert_eq!(s.history.undo.len(), undo + 1);
    assert_eq!(s.project.transcripts[&item], before.transcripts[&item]);
    assert_eq!(q.duration(), old.duration() - total);
    assert_eq!(q.markers[0].start, old.markers[0].start - total);
    assert_eq!(q.caption_tracks.len(), 2);
    for (actual, original) in q.caption_tracks.iter().zip(&old.caption_tracks) {
        assert_eq!(actual.name, original.name);
        assert_eq!(actual.style, original.style);
        assert_eq!(actual.captions.iter().map(|c| &c.text).collect::<Vec<_>>(), original.captions.iter().map(|c| &c.text).collect::<Vec<_>>());
    }
    assert_eq!(crate::transcript::sequence_words(&s).into_iter().map(|w| w.text).collect::<Vec<_>>(), words);
    q.check().unwrap();
    let after = s.project.clone();
    assert!(s.execute("gmcut.apply", json!({})).is_err(), "same plan cannot apply twice");
    assert!(Arc::ptr_eq(&after, &s.project));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project, before);
}
#[test]
fn invalid_settings_worker_output_and_stale_results_never_mutate_project() {
    let (mut s, _) = session(false);
    let before = s.project.clone();
    for p in [json!({"minSeconds":0.01}), json!({"keepSeconds":-1}), json!({"presets":[0,0.2]}), json!({"sensitivity":"bad"})] {
        assert!(s.execute("gmcut.analyze", p).is_err());
        assert!(Arc::ptr_eq(&before, &s.project));
    }
    let report = analyze(&mut s);
    assert_eq!(report["fresh"], true);
    for (command, p) in [("gmcut.choose", json!({"preset":0})), ("gmcut.move", json!({"index":999})), ("gmcut.auto", json!({"keepSeconds":-1}))] {
        assert!(s.execute(command, p).is_err());
        assert_eq!(s.execute("gmcut.inspect", json!({})).unwrap(), report);
    }
    s.execute("gmcut.auto", json!({})).unwrap();
    s.path = Some("another.fcproj".into());
    assert!(s.execute("gmcut.apply", json!({})).is_err());
    assert!(Arc::ptr_eq(&before, &s.project));
    s.execute("gmcut.reset", json!({})).unwrap();
    let mut invalid = values();
    invalid[0].end = sec(20.);
    s.transcriber = Some(Arc::new(Fixed { candidates: invalid, fail: false }));
    let report = analyze(&mut s);
    assert_eq!(report["fresh"], false);
    assert_eq!(report["running"], false);
    assert!(Arc::ptr_eq(&before, &s.project));
}
#[test]
fn locked_tracks_and_asr_word_conflicts_refuse_apply_atomically() {
    for mode in 0..4 {
        let (mut s, item) = session(true);
        if mode == 3 {
            let mut t = (*s.project.transcripts[&item]).clone();
            t.words[0].end = sec(1.5);
            s.execute("transcript.set", json!({"item":item.0,"transcript":t})).unwrap();
        } else {
            s.edit_sequence("Synthetic track protection", |q, _, _| {
                match mode {
                    0 => q.audio_tracks[0].locked = true,
                    1 => q.caption_tracks[0].sync_lock = false,
                    _ => q.video_tracks[0].locked = true,
                }
                Ok(())
            })
            .unwrap();
        }
        let before = s.project.clone();
        let undo = s.history.undo.len();
        analyze(&mut s);
        s.execute("gmcut.auto", json!({})).unwrap();
        assert!(s.execute("gmcut.apply", json!({})).is_err());
        assert!(Arc::ptr_eq(&before, &s.project));
        assert_eq!(s.history.undo.len(), undo);
    }
}
#[test]
fn simple_audio_only_is_supported_but_trimmed_and_multiple_clips_are_not() {
    let (mut s, _) = session(false);
    s.edit_sequence("Audio-only fixture", |q, _, _| {
        for t in &mut q.video_tracks {
            t.items.clear();
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(analyze(&mut s)["fresh"], true);
    s.execute("gmcut.reset", json!({})).unwrap();
    s.edit_sequence("Trim sample", |q, _, _| {
        q.audio_tracks[0].items[0].source_in = sec(1.);
        Ok(())
    })
    .unwrap();
    let before = s.project.clone();
    assert!(s.execute("gmcut.analyze", json!({})).is_err());
    assert!(Arc::ptr_eq(&before, &s.project));
}

#[test]
fn saved_standalone_32_cut_plan_matches_exact_native_frame_indices() {
    // Golden source-time decisions only; no media path, decoding, model or export is used.
    let rows: &[(f64, f64, f64, f64, bool, i64, i64)] = &[
        (2.96, 3.22, 2.88, 3.26, false, 87, 97),
        (6.55, 6.77, 6.47, 6.81, false, 194, 204),
        (10.58, 10.84, 10.5, 10.88, false, 315, 326),
        (14.32, 14.48, 14.24, 14.52, false, 427, 435),
        (17.81, 17.94, 17.73, 17.98, false, 532, 538),
        (25.01, 25.27, 24.93, 25.31, false, 748, 758),
        (27.56, 27.71, 27.56, 27.71, true, 826, 830),
        (31.01, 31.07, 31.01, 31.07, true, 930, 931),
        (33.52, 33.65, 33.44, 33.69, false, 1003, 1009),
        (38.55, 38.71, 38.47, 38.75, false, 1153, 1161),
        (41.04, 41.36, 40.96, 41.4, false, 1228, 1240),
        (44.44, 44.79, 44.36, 44.83, false, 1330, 1343),
        (50.02, 50.17, 50.02, 50.17, true, 1500, 1503),
        (54.28, 54.43, 54.28, 54.43, true, 1627, 1631),
        (56.37, 56.6, 56.29, 56.64, false, 1688, 1697),
        (60.02, 60.44, 59.94, 60.48, false, 1797, 1812),
        (63.01, 63.16, 63.01, 63.16, true, 1889, 1892),
        (64.88, 65.01, 64.8, 65.05, false, 1943, 1949),
        (67.25, 67.7, 67.17, 67.74, false, 2014, 2030),
        (69.84, 70.2, 69.76, 70.24, false, 2091, 2105),
        (74.52, 74.74, 74.44, 74.78, false, 2231, 2241),
        (77.04, 77.14, 76.96, 77.18, false, 2307, 2313),
        (83.22, 83.64, 83.14, 83.68, false, 2492, 2507),
        (87.06, 87.19, 86.98, 87.23, false, 2607, 2614),
        (90.42, 90.74, 90.34, 90.78, false, 2708, 2720),
        (93.56, 93.91, 93.48, 93.95, false, 2802, 2815),
        (96.29, 96.44, 96.29, 96.44, true, 2886, 2890),
        (98.28, 98.4, 98.28, 98.4, true, 2946, 2949),
        (105.62, 105.72, 105.54, 105.76, false, 3164, 3169),
        (109.43, 109.49, 109.35, 109.53, false, 3278, 3282),
        (113.0, 113.08, 113.0, 113.08, true, 3387, 3389),
        (113.88, 114.01, 113.8, 114.01, false, 3411, 3416),
    ];
    let rate = FrameRate::FPS_29_97;
    let mut total = Tick::ZERO;
    for &(start, end, pause_start, pause_end, whole_only, first, last) in rows {
        let c = Candidate {
            start: sec(start),
            end: sec(end),
            pause_start: sec(pause_start),
            pause_end: sec(pause_end),
            whole_only,
            keep: Some(Tick::ZERO),
            skipped: false,
        };
        let r = cut(&c, Tick::ZERO, rate).unwrap();
        assert_eq!(r.start, rate.tick_of(first));
        assert_eq!(r.end(), rate.tick_of(last));
        total += r.duration;
    }
    assert_eq!(rows.len(), 32);
    assert!((total - sec(8.5085)).abs() <= Tick(1));
}

#[test]
fn a_plan_cannot_delete_the_entire_source() {
    let (mut s, _) = session(false);
    let duration = s.active_sequence().unwrap().duration();
    s.transcriber = Some(Arc::new(Fixed {
        candidates: vec![GmCutCandidate { start: Tick::ZERO, end: duration, pause_start: Tick::ZERO, pause_end: duration, whole_only: true }],
        fail: false,
    }));
    let before = s.project.clone();
    assert_eq!(analyze(&mut s)["fresh"], true);
    s.execute("gmcut.auto", json!({})).unwrap();
    assert!(s.execute("gmcut.apply", json!({})).is_err());
    assert!(Arc::ptr_eq(&before, &s.project));
}

#[test]
fn analysis_mono_preserves_mono_and_equal_power_stereo_levels() {
    let mono = vec![0.25, -0.5, 1.0];
    assert_eq!(analysis_mono(std::slice::from_ref(&mono)).unwrap(), mono);
    let stereo = analysis_mono(&[vec![0.25, -0.5, 0.25], vec![0.25, -0.5, 0.0]]).unwrap();
    for (actual, expected) in stereo.iter().zip([0.25 * std::f32::consts::SQRT_2, -0.5 * std::f32::consts::SQRT_2, 0.25 * std::f32::consts::FRAC_1_SQRT_2]) {
        assert!((actual - expected).abs() < 1e-7, "stereo analysis must not lose 3 dB");
    }
    assert!(analysis_mono(&[vec![0.0], vec![], vec![0.0]]).is_err());
    assert!(analysis_mono(&[vec![0.0], vec![]]).is_err());
}
