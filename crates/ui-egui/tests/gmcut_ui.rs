//! Native GM컷 workflow using generated media and a fixed analyzer; no personal files or ASR.
use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::Transcript;
use filmcraft_speech::{GmCutCandidate, Options, ProgressFn, SpeechError, Transcriber};
use filmcraft_time::{FrameRate, Tick};
use filmcraft_ui_egui::{AudioOut, FilmcraftApp, control::ControlRequest, dock::PanelKind};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{Sender, channel},
};

fn sec(value: f64) -> Tick {
    Tick::from_seconds_f64(value)
}
struct Analyzer;
impl Transcriber for Analyzer {
    fn id(&self) -> String {
        "gmcut-ui-fixture".into()
    }
    fn transcribe(&self, _: &[f32], _: &Options, _: ProgressFn) -> Result<Transcript, SpeechError> {
        Ok(Transcript::default())
    }
    fn gmcut_candidates(&self, _: &[f32], _: f64, _: &str, _: FrameRate, _: ProgressFn) -> Result<Vec<GmCutCandidate>, SpeechError> {
        Ok(vec![
            GmCutCandidate { start: sec(0.78), end: sec(1.96), pause_start: sec(0.7), pause_end: sec(2.0), whole_only: false },
            GmCutCandidate { start: sec(2.48), end: sec(3.76), pause_start: sec(2.4), pause_end: sec(3.8), whole_only: false },
        ])
    }
}
struct ClockAudio(Arc<AtomicU64>);
impl AudioOut for ClockAudio {
    fn start(&mut self, _: Box<dyn FnMut(&mut [f32], usize) + Send>) -> Result<u32, String> {
        self.0.store(0, Ordering::Relaxed);
        Ok(48000)
    }
    fn stop(&mut self) {}
    fn sample_rate(&self) -> u32 {
        48000
    }
    fn played_frames(&self) -> Option<u64> {
        Some(self.0.load(Ordering::Relaxed))
    }
}
struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    menu_tx: Sender<String>,
    clock: Arc<AtomicU64>,
}
impl Driver {
    fn new() -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        session
            .edit_sequence("Isolate native GM컷 sample", |sequence, _, _| {
                let duration = sequence.settings.frame_rate.tick_of(120);
                for (index, track) in sequence.audio_tracks.iter_mut().enumerate() {
                    track.items.truncate(usize::from(index == 0));
                    track.transitions.clear();
                    for clip in &mut track.items {
                        clip.start = Tick::ZERO;
                        clip.source_in = Tick::ZERO;
                        clip.duration = duration;
                    }
                }
                for (index, track) in sequence.video_tracks.iter_mut().enumerate() {
                    track.items.truncate(usize::from(index == 0));
                    track.transitions.clear();
                    for clip in &mut track.items {
                        clip.start = Tick::ZERO;
                        clip.source_in = Tick::ZERO;
                        clip.duration = duration;
                    }
                }
                sequence.caption_tracks.clear();
                Ok(())
            })
            .unwrap();
        session.transcriber = Some(Arc::new(Analyzer));
        session.prefs.audio.scrub_audio = true;
        let clock = Arc::new(AtomicU64::new(0));
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(session).with_control(rx);
        let (menu_tx, menu_rx) = channel();
        app.command_inbox = Some(menu_rx);
        app.audio = Some(Box::new(ClockAudio(clock.clone())));
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 1100.0)).with_max_steps(10000).build_eframe(move |_| app);
        let mut driver = Self { harness, tx, menu_tx, clock };
        driver.frames(4);
        driver
    }
    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }
    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (request, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(request).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(value) = reply.try_recv() {
                assert_eq!(value["ok"], true, "{method} {params}: {value}");
                return value["result"].clone();
            }
        }
        panic!("control timeout: {method}");
    }
    fn report(&mut self) -> Value {
        self.ok("engine.execute", json!({"command":"gmcut.inspect","params":{}}))
    }
    fn key(&mut self, key: &str) {
        self.ok("ui.key", json!({"key":key}));
        self.frames(2);
    }
    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id":id}));
        self.frames(2);
    }
}
#[test]
fn native_gmcut_review_uses_audio_clock_and_preserves_source_and_shortcuts() {
    let mut d = Driver::new();
    d.ok("ui.set", json!({"workspace":"GM컷","focused":"GmcutReview"}));
    for panel in [PanelKind::Project, PanelKind::Program, PanelKind::Gmcut, PanelKind::Timeline, PanelKind::GmcutReview] {
        assert!(d.harness.state().ui.dock.is_visible(panel));
    }
    assert_eq!(d.harness.state().ui.gmcut.min_seconds, 0.05);
    assert_eq!(d.harness.state().ui.gmcut.presets, [0., 0.2, 0.3]);
    let original = d.harness.state().session.project.clone();
    let history = d.harness.state().session.history.undo.len();
    d.click("gmcut.analyze");
    for _ in 0..600 {
        if d.report()["running"] == false {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(d.report()["fresh"], true);
    assert_eq!(d.report()["count"], 2);
    d.ok("ui.set", json!({"focused":"Timeline"}));
    d.ok("ui.focus", json!({"id":"dock.gutter.a"}));
    let before_resize = serde_json::to_value(&d.harness.state().ui.dock).unwrap();
    d.key("ArrowRight");
    assert_eq!(d.report()["cursor"], 0, "0.10 dock dividers keep their arrow-key resize behavior");
    assert_ne!(serde_json::to_value(&d.harness.state().ui.dock).unwrap(), before_resize);
    d.harness.ctx.memory_mut(|memory| memory.stop_text_input());
    d.ok("ui.set", json!({"focused":"GmcutReview"}));
    d.click("gmcut.choose2");
    assert_eq!(d.report()["cursor"], 1);
    assert_eq!(d.report()["candidates"][0]["keepSeconds"], 0.2);
    assert!(d.harness.state().ui.gmcut.preview.is_some(), "preview must survive frame-grid snapping: {}", d.harness.state().ui.gmcut.status);
    let preview_window = d.report()["preview"].clone();
    let left_start = Tick(preview_window["leftStart"].as_i64().unwrap());
    assert_ne!(left_start, d.harness.state().session.sequence_rate().snap(left_start), "fixture exercises an unsnapped 0.8s context window");
    let before = d.harness.state().session.playhead();
    d.clock.store(2400, Ordering::Relaxed);
    d.harness.input_mut().events.push(egui::Event::PointerButton {
        pos: egui::pos2(3.0, 3.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    d.frames(2);
    assert_eq!(d.clock.load(Ordering::Relaxed), 2400, "scrubbing must not replace the active GM컷 preview audio stream");
    d.harness.input_mut().events.push(egui::Event::PointerButton {
        pos: egui::pos2(3.0, 3.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    d.frames(1);
    assert!(d.harness.state().session.playhead() > before, "Program monitor advances with consumed audio samples");
    let preview = d.report()["preview"].clone();
    let left_seconds = (preview["leftEnd"].as_i64().unwrap() - preview["leftStart"].as_i64().unwrap()) as f64 / filmcraft_time::TICKS_PER_SECOND as f64;
    d.clock.store(((left_seconds + 0.05) * 48000.) as u64, Ordering::Relaxed);
    d.frames(2);
    assert!(d.harness.state().session.playhead().0 >= preview["rightStart"].as_i64().unwrap(), "video skips the same source cut as audio");
    d.harness.state_mut().session.set_playhead(sec(4.5));
    d.frames(2);
    assert!(d.harness.state().ui.gmcut.preview.is_none(), "manual seeking cancels the what-if preview");
    d.ok("ui.set", json!({"focused":"GmcutReview"}));
    d.key("P");
    assert!(d.harness.state().ui.gmcut.preview.is_some());
    d.harness.state_mut().toggle_play(1.0);
    assert!(d.harness.state().ui.gmcut.preview.is_none());
    assert!(!d.harness.state().playback.playing, "Space stops an active what-if preview");
    d.key("P");
    d.harness.state_mut().play(-1.0);
    assert!(d.harness.state().ui.gmcut.preview.is_none(), "normal shuttle playback cancels the what-if preview");
    d.harness.state_mut().stop();
    d.ok("engine.execute", json!({"command":"gmcut.choose","params":{"preset":3}}));
    let saved_history = std::mem::take(&mut d.harness.state_mut().session.history.undo);
    assert!(
        filmcraft_ui_egui::menus::menu_items(d.harness.state()).iter().any(|item| item.id == "edit.undo" && item.enabled),
        "native Undo stays enabled with only review history"
    );
    d.menu_tx.send("edit.undo".into()).unwrap();
    d.harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    d.frames(3);
    assert_eq!(d.report()["candidates"][0]["keepSeconds"], 0.2, "one native accelerator must undo only one review choice");
    assert!(d.report()["candidates"][1]["keepSeconds"].is_null());
    assert!(Arc::ptr_eq(&original, &d.harness.state().session.project), "native CmdZ must never undo sequence creation while reviewing");
    assert!(d.harness.state().session.history.undo.is_empty());
    d.harness.state_mut().session.history.undo = saved_history;
    d.key("Cmd+Z");
    assert!(d.report()["candidates"][0]["keepSeconds"].is_null());
    d.menu_tx.send("edit.undo".into()).unwrap();
    d.frames(3);
    d.key("Cmd+Z");
    assert!(Arc::ptr_eq(&original, &d.harness.state().session.project), "exhausted review Undo must not fall through to project Undo");
    d.ok("ui.click", json!({"id":"gmcut.minSeconds","count":2}));
    d.key("1");
    assert!(d.report()["candidates"][0]["keepSeconds"].is_null(), "typed digits never choose a cut");
    d.menu_tx.send("edit.undo".into()).unwrap();
    d.frames(3);
    assert!(Arc::ptr_eq(&original, &d.harness.state().session.project), "native Undo in a text field never changes the project");
    d.harness.ctx.memory_mut(|memory| memory.stop_text_input());
    d.ok("ui.set", json!({"focused":"GmcutReview","gmcut":{"minSeconds":0.05}}));
    d.key("Shift+3");
    assert_eq!(d.harness.state().ui.focused, PanelKind::Timeline, "existing modified shortcut remains active");
    d.key("P");
    let duration = d.report()["preview"]["frames"].as_u64().unwrap();
    let sr = d.report()["preview"]["sampleRate"].as_u64().unwrap();
    d.clock.store(duration * 48000 / sr + 48000, Ordering::Relaxed);
    d.frames(3);
    assert_eq!(d.report()["cursor"], 1, "P previews then moves to the next candidate");
    d.key("ArrowLeft");
    d.key("3");
    d.key("S");
    assert!(d.harness.state().ui.gmcut.preview.is_none());
    assert_eq!(d.report()["candidates"][1]["skipped"], true);
    assert!(Arc::ptr_eq(&original, &d.harness.state().session.project));
    assert_eq!(d.harness.state().session.history.undo.len(), history);
    d.ok("ui.scroll", json!({"id":"panel.Gmcut","dy":-600}));
    d.frames(4);
    d.click("gmcut.apply");
    assert_eq!(d.harness.state().session.history.undo.len(), history + 1);
    d.menu_tx.send("edit.undo".into()).unwrap();
    d.frames(3);
    assert_eq!(d.harness.state().session.project, original, "after explicit apply, standard project undo restores the timeline");

    // The dedicated import starts a new audio-only sequence and keeps the previous project sequence.
    let old_sequence = d.harness.state().session.state.active_sequence.unwrap();
    let old_contents = d.harness.state().session.active_sequence().unwrap().clone();
    let wave = std::env::temp_dir().join(format!("filmcraft-gmcut-ui-{}.wav", std::process::id()));
    let sample_count = 16000u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + sample_count * 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&16000u32.to_le_bytes());
    bytes.extend_from_slice(&32000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(sample_count * 2).to_le_bytes());
    bytes.resize(44 + sample_count as usize * 2, 0);
    std::fs::write(&wave, bytes).unwrap();
    d.ok("ui.set", json!({"gmcut":{"minSeconds":0.6,"sensitivity":"sensitive","keepSeconds":0.3,"presets":[0.0,0.4,0.8]}}));
    d.ok("engine.execute", json!({"command":"gmcut.import","params":{"path":wave.to_string_lossy()}}));
    assert_ne!(d.harness.state().session.state.active_sequence, Some(old_sequence));
    assert_eq!(d.harness.state().ui.gmcut.min_seconds, 0.05);
    assert_eq!(d.harness.state().ui.gmcut.keep_seconds, 0.0);
    assert_eq!(d.harness.state().ui.gmcut.sensitivity, "normal");
    assert_eq!(d.harness.state().ui.gmcut.presets, [0.0, 0.4, 0.8]);
    let new_sequence = d.harness.state().session.active_sequence().unwrap();
    assert_eq!(new_sequence.audio_tracks.iter().map(|track| track.items.len()).sum::<usize>(), 1);
    assert_eq!(d.harness.state().session.project.sequence(old_sequence), Some(&old_contents));
    std::fs::remove_file(wave).unwrap();
}

#[test]
fn older_ui_state_adds_gmcut_defaults_without_changing_existing_preferences() {
    let original = filmcraft_ui_egui::state::UiState {
        workspace: "Color".into(),
        text_tab: "Captions".into(),
        caption_search: "기존 자막 검색".into(),
        ..Default::default()
    };
    let mut saved = serde_json::to_value(&original).unwrap();
    saved.as_object_mut().unwrap().remove("gmcut");
    let restored: filmcraft_ui_egui::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.workspace, original.workspace);
    assert_eq!(restored.dock, original.dock);
    assert_eq!(restored.text_tab, original.text_tab);
    assert_eq!(restored.caption_search, original.caption_search);
    assert_eq!(restored.gmcut.min_seconds, 0.05);
    assert_eq!(restored.gmcut.presets, [0.0, 0.2, 0.3]);
}
