//! GM컷 shares FilmCraft's source, Program monitor, timeline and undo engine.
use egui::{Rect, UiBuilder};
use filmcraft_project::ItemId;
use filmcraft_time::{Tick, TimeDisplay, format_time};
use serde_json::{Value, json};
use web_time::Instant;

use crate::{FilmcraftApp, dock::PanelKind};

#[derive(Clone, Debug)]
pub struct PreviewPlayback {
    started: Instant,
    left_start: Tick,
    left_end: Tick,
    right_start: Tick,
    right_end: Tick,
    last_tick: Tick,
    seconds: f64,
    next: usize,
    sequence: Option<ItemId>,
    revision: u64,
}

fn inspect(app: &mut FilmcraftApp) -> Value {
    match app.session.execute("gmcut.inspect", json!({})) {
        Ok(value) => value,
        Err(error) => {
            app.ui.gmcut.status = error.to_string();
            Value::Null
        }
    }
}
fn configured(app: &FilmcraftApp, plan: &Value) -> bool {
    let same = |value: &Value, seconds: f64| value.as_f64().is_some_and(|value| (value - seconds).abs() < 1e-9);
    same(&plan["minSeconds"], app.ui.gmcut.min_seconds)
        && same(&plan["keepSeconds"], app.ui.gmcut.keep_seconds)
        && plan["sensitivity"].as_str() == Some(app.ui.gmcut.sensitivity.as_str())
        && plan["presets"]
            .as_array()
            .is_some_and(|values| values.len() == 3 && values.iter().zip(app.ui.gmcut.presets).all(|(value, preset)| same(value, preset)))
}
fn update(app: &mut FilmcraftApp, ctx: &egui::Context) -> Value {
    let mut plan = inspect(app);
    if plan["running"].as_bool() == Some(true) {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    } else if app.ui.gmcut.auto_pending {
        app.ui.gmcut.auto_pending = false;
        if plan["fresh"].as_bool() == Some(true) && configured(app, &plan) {
            run(app, ctx, "gmcut.auto", json!({"keepSeconds":app.ui.gmcut.keep_seconds}));
            plan = inspect(app);
        }
    }
    plan
}
fn button(app: &mut FilmcraftApp, ui: &mut egui::Ui, id: &str, label: &str, enabled: bool) -> bool {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    app.auto.add_response(&format!("gmcut.{id}"), &response, label);
    response.clicked()
}
fn start(app: &mut FilmcraftApp, ctx: &egui::Context, automatic: bool) {
    app.stop();
    let params = json!({"minSeconds":app.ui.gmcut.min_seconds,"keepSeconds":app.ui.gmcut.keep_seconds,"presets":app.ui.gmcut.presets,"sensitivity":app.ui.gmcut.sensitivity});
    match crate::menus::invoke(app, ctx, "gmcut.analyze", params) {
        Ok(_) => {
            app.ui.gmcut.auto_pending = automatic;
            app.ui.gmcut.status = "안전 무음 분석 중 · 원본과 타임라인은 유지됩니다".into();
        }
        Err(error) => {
            app.ui.gmcut.auto_pending = false;
            app.ui.gmcut.status = error;
        }
    }
}
fn run(app: &mut FilmcraftApp, ctx: &egui::Context, command: &str, params: Value) {
    let selected = (command == "gmcut.choose").then(|| inspect(app)["cursor"].clone());
    if command != "playback.gmcutPreview" {
        app.stop();
    }
    match crate::menus::invoke(app, ctx, command, params) {
        Ok(_) => {
            app.ui.gmcut.status = match command {
                "gmcut.apply" => "타임라인 반영 완료 · ⌘Z로 전체 반영을 되돌릴 수 있습니다".into(),
                "gmcut.import" => app.ui.gmcut.status.clone(),
                "gmcut.auto" => "자동 정리안이 준비됐습니다. 후보를 검토하고 타임라인 반영을 눌러 완료하세요".into(),
                "gmcut.skip" => "이 무음은 원래 길이로 보존합니다".into(),
                _ => String::new(),
            };
            if let Some(index) = selected
                && let Err(error) = preview(app, ctx, &json!({"index":index}))
            {
                app.ui.gmcut.status = format!("유지 시간 선택 완료 · 미리보기: {error}");
            }
        }
        Err(error) => app.ui.gmcut.status = error,
    }
    if !app.ui.gmcut.status.is_empty() {
        app.ui.status.clone_from(&app.ui.gmcut.status);
    }
}

pub fn settings(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let plan = update(app, ui.ctx());
    let running = plan["running"].as_bool().unwrap_or(false);
    let fresh = plan["fresh"].as_bool().unwrap_or(false);
    let mut action = None;
    let mut analyze = None;
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink(12.0)).id_salt("gmcut-settings"));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut child, |ui| {
        ui.heading("GM컷");
        ui.label("말 사이 호흡을 빠르게 정리하세요");
        if button(app, ui, "import", "영상·음성 가져오기", !running) {
            action = Some(("gmcut.import", json!({})));
        }
        ui.small("가져온 소스로 새 시퀀스를 준비합니다. 기존 시퀀스는 보존됩니다.");
        ui.add_space(10.0);
        ui.add_enabled_ui(!running, |ui| {
            ui.label("검출할 최소 무음");
            let response = ui.add(egui::DragValue::new(&mut app.ui.gmcut.min_seconds).speed(0.01).range(0.05..=20.0).fixed_decimals(2).suffix("초"));
            app.auto.add_response("gmcut.minSeconds", &response, "검출할 최소 무음");
            ui.add_space(8.0);
            ui.label("민감도");
            ui.horizontal_wrapped(|ui| {
                for (value, label) in [("quiet", "조용함"), ("normal", "보통"), ("sensitive", "민감")] {
                    let response = ui.selectable_label(app.ui.gmcut.sensitivity == value, label);
                    app.auto.add_response(&format!("gmcut.sensitivity.{value}"), &response, label);
                    if response.clicked() {
                        app.ui.gmcut.sensitivity = value.into();
                    }
                }
            });
            ui.add_space(8.0);
            ui.label("자동 정리 · 유지 시간");
            let response = ui.add(egui::DragValue::new(&mut app.ui.gmcut.keep_seconds).speed(0.01).range(0.0..=10.0).fixed_decimals(2).suffix("초"));
            app.auto.add_response("gmcut.keepSeconds", &response, "자동 정리 유지 시간");
            ui.small("0초는 검출한 안전 무음을 모두 제거합니다. 유지 시간은 양쪽에 남기는 무음의 합입니다.");
            ui.add_space(8.0);
            for index in 0..3 {
                ui.horizontal(|ui| {
                    ui.label(format!("{}번 유지", index + 1));
                    if let Some(seconds) = app.ui.gmcut.presets.get_mut(index) {
                        let response = ui.add(egui::DragValue::new(seconds).speed(0.01).range(0.0..=10.0).fixed_decimals(2).suffix("초"));
                        app.auto.add_response(&format!("gmcut.preset{}.seconds", index + 1), &response, "키 유지 시간");
                    }
                });
            }
        });
        ui.separator();
        let loaded = app.session.active_sequence().is_some();
        if button(app, ui, "auto", "자동으로 정리", loaded && !running) {
            analyze = Some(true);
        }
        if button(app, ui, "analyze", "무음 후보 검토", loaded && !running) {
            analyze = Some(false);
        }
        if running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("파형과 발화를 분석하고 있습니다");
            });
        }
        if let Some(status) = plan["status"].as_str()
            && !status.is_empty()
        {
            ui.label(status);
        }
        if fresh && !configured(app, &plan) {
            ui.colored_label(app.tokens.hot_text, "설정이 바뀌었습니다. 다시 분석하면 새 설정으로 검토할 수 있습니다.");
        }
        let count = plan["count"].as_u64().unwrap_or(0);
        let reviewed = plan["reviewed"].as_u64().unwrap_or(0);
        ui.label(format!("무음 {count}개 · 검토 {reviewed}개"));
        ui.label(format!("줄어드는 시간 {:.2}초", plan["removeSeconds"].as_f64().unwrap_or(0.0)));
        let choices = plan["candidates"].as_array().is_some_and(|values| values.iter().any(|candidate| candidate["cut"].is_object()));
        if button(app, ui, "apply", "타임라인 반영", fresh && !running && configured(app, &plan) && choices) {
            action = Some(("gmcut.apply", json!({})));
        }
        ui.small("검토안은 원본을 바꾸지 않습니다. 반영하면 기존 타임라인과 자막을 함께 정리하며 한 번에 실행취소할 수 있습니다.");
        if !app.ui.gmcut.status.is_empty() {
            ui.separator();
            ui.label(&app.ui.gmcut.status);
        }
    });
    if let Some(automatic) = analyze {
        start(app, ui.ctx(), automatic);
    }
    if let Some((command, params)) = action {
        run(app, ui.ctx(), command, params);
    }
}

pub fn review(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let plan = update(app, ui.ctx());
    let candidates = plan["candidates"].as_array();
    let count = candidates.map(Vec::len).unwrap_or(0);
    let cursor = plan["cursor"].as_u64().and_then(|value| usize::try_from(value).ok()).unwrap_or(0);
    let ready = plan["fresh"].as_bool() == Some(true) && plan["running"].as_bool() != Some(true) && configured(app, &plan);
    let mut action = None;
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink(10.0)).id_salt("gmcut-review"));
    child.horizontal_wrapped(|ui| {
        ui.strong(if cursor < count {
            format!("GM컷 · {:02} / {:02}", cursor + 1, count)
        } else if count > 0 {
            format!("GM컷 · 검토 완료 {count}개")
        } else {
            "GM컷 · 무음 후보를 분석하세요".into()
        });
        if let Some(candidate) = candidates.and_then(|values| values.get(cursor)) {
            let rate = app.session.sequence_rate();
            let start = Tick(candidate["pauseStart"].as_i64().or_else(|| candidate["start"].as_i64()).unwrap_or(0));
            ui.label(format_time(start, rate, false, TimeDisplay::Timecode, 48_000));
            if candidate["wholeOnly"].as_bool() == Some(true) {
                ui.label("짧은 무음 · 전체 제거 또는 보존");
            }
        }
        if app.ui.gmcut.preview.is_some() {
            ui.colored_label(app.tokens.hot_text, "편집 영상·음성 미리보기 → 다음 후보");
        }
    });
    child.horizontal_wrapped(|ui| {
        for (index, seconds) in app.ui.gmcut.presets.into_iter().enumerate() {
            let label = if seconds == 0.0 { format!("{}  전체", index + 1) } else { format!("{}  {:.2}초", index + 1, seconds) };
            if button(app, ui, &format!("choose{}", index + 1), &label, ready && cursor < count) {
                action = Some(("gmcut.choose", json!({"preset":index + 1})));
            }
        }
        if button(app, ui, "previous", "← 이전", ready && cursor > 0) {
            action = Some(("gmcut.move", json!({"index":cursor.saturating_sub(1)})));
        }
        if button(app, ui, "next", "다음 →", ready && cursor.saturating_add(1) < count) {
            action = Some(("gmcut.move", json!({"index":cursor.saturating_add(1)})));
        }
        if button(app, ui, "skip", "S 보존", ready && cursor < count) {
            action = Some(("gmcut.skip", json!({})));
        }
        if button(app, ui, "undo", "⌘Z 실행취소", ready && plan["reviewed"].as_u64().unwrap_or(0) > 0) {
            action = Some(("gmcut.undo", json!({})));
        }
        if button(app, ui, "preview", "P 편집 미리보기", ready && count > 0) {
            action = Some(("playback.gmcutPreview", json!({"index":if cursor < count {cursor} else {0}})));
        }
        if button(app, ui, "original", "원본 보기", ready && cursor < count) {
            action = Some(("playback.gmcutOriginal", json!({})));
        }
        if button(app, ui, "stop", "정지", true) {
            app.stop();
        }
    });
    child.small("1 / 2 / 3 유지 시간 선택 · P 영상·음성 미리보기 후 다음 · ← / → 이동 · S 보존 · 숫자 입력 중에는 단축키가 동작하지 않습니다.");
    if let Some((command, params)) = action {
        run(app, ui.ctx(), command, params);
    }
}

/// Dedicated import creates a new single-source sequence, preserving existing sequences.
pub fn import_media(app: &mut FilmcraftApp, params: &Value) -> Result<Value, String> {
    if let Some(path) = params.get("path").and_then(Value::as_str) {
        return import_path(app, path);
    }
    let extensions: Vec<&str> = filmcraft_media::VIDEO_EXTENSIONS.iter().chain(filmcraft_media::AUDIO_EXTENSIONS).copied().collect();
    app.pick(crate::native_dialogs::FileDialog::open_file("영상·음성", &extensions), |app, paths| match paths.first() {
        Some(path) => import_path(app, path),
        None => Ok(Value::Null),
    })
}
fn import_path(app: &mut FilmcraftApp, path: &str) -> Result<Value, String> {
    app.stop();
    let imported = app.session.execute("file.import", json!({"paths":[path],"bin":app.import_bin().0})).map_err(|error| error.to_string())?;
    let item = imported["items"].as_array().and_then(|items| items.first()).and_then(Value::as_u64).ok_or_else(|| {
        imported["errors"]
            .as_array()
            .map(|errors| errors.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "))
            .filter(|error| !error.is_empty())
            .unwrap_or_else(|| "가져올 영상·음성 파일이 없습니다".into())
    })?;
    let sequence = app.session.execute("file.newSequenceFromClip", json!({"items":[item]})).map_err(|error| error.to_string())?;
    let presets = app.ui.gmcut.presets;
    app.ui.gmcut = crate::state::GmcutUi { presets, ..Default::default() };
    app.ui.gmcut.status = "영상이 준비됐습니다. 자동으로 정리 또는 무음 후보 검토를 눌러 시작하세요".into();
    app.set_workspace("GM컷");
    Ok(sequence)
}

pub fn preview(app: &mut FilmcraftApp, ctx: &egui::Context, params: &Value) -> Result<Value, String> {
    app.stop();
    let plan = app.session.execute("gmcut.inspect", json!({})).map_err(|error| error.to_string())?;
    let index = params
        .get("index")
        .or_else(|| plan.get("cursor"))
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or("잘못된 후보 번호입니다")?;
    let result = app.session.execute("gmcut.preview", json!({"index":index})).map_err(|error| error.to_string())?;
    let tick = |name: &str| result[name].as_i64().map(Tick).ok_or_else(|| format!("미리보기 {name} 시간이 없습니다"));
    let (left_start, left_end, right_start, right_end) = (tick("leftStart")?, tick("leftEnd")?, tick("rightStart")?, tick("rightEnd")?);
    if left_start > left_end || left_end > right_start || right_start > right_end {
        return Err("미리보기 시간 범위가 올바르지 않습니다".into());
    }
    let seconds = app.play_gmcut_preview()?;
    let cursor = plan["cursor"].as_u64().and_then(|value| usize::try_from(value).ok()).unwrap_or(index);
    let count = plan["count"].as_u64().and_then(|value| usize::try_from(value).ok()).unwrap_or(0);
    let next = if cursor > index && cursor < count { cursor } else { index.saturating_add(1) };
    app.session.set_playhead(left_start);
    app.ui.gmcut.preview = Some(PreviewPlayback {
        started: Instant::now(),
        left_start,
        left_end,
        right_start,
        right_end,
        last_tick: app.session.playhead(),
        seconds,
        next,
        sequence: app.session.state.active_sequence,
        revision: app.session.revision,
    });
    ctx.request_repaint();
    Ok(result)
}

pub fn advance_preview(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(mut preview) = app.ui.gmcut.preview.take() else { return };
    if app.session.playhead() != preview.last_tick || app.session.state.active_sequence != preview.sequence || app.session.revision != preview.revision {
        app.stop();
        return;
    }
    if let Some(audio) = app.audio.as_mut() {
        audio.recover();
    }
    let elapsed = app
        .audio
        .as_ref()
        .and_then(|audio| audio.played_frames().filter(|_| audio.sample_rate() > 0).map(|frames| frames as f64 / f64::from(audio.sample_rate())))
        .unwrap_or_else(|| preview.started.elapsed().as_secs_f64());
    if elapsed >= preview.seconds {
        app.stop();
        app.session.set_playhead(preview.right_end);
        let plan = inspect(app);
        if plan["candidates"].as_array().is_some_and(|candidates| preview.next < candidates.len()) {
            run(app, ctx, "gmcut.move", json!({"index":preview.next}));
        }
        return;
    }
    let elapsed_ticks = Tick::from_seconds_f64(elapsed.max(0.0));
    let before = preview.left_end - preview.left_start;
    let at = if elapsed_ticks < before { preview.left_start + elapsed_ticks } else { preview.right_start + elapsed_ticks - before }.min(preview.right_end);
    app.session.set_playhead(at);
    // set_playhead snaps to the sequence frame grid; compare that displayed tick on the next pass.
    preview.last_tick = app.session.playhead();
    app.ui.gmcut.preview = Some(preview);
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
}

pub fn original(app: &mut FilmcraftApp, params: &Value) -> Result<Value, String> {
    app.stop();
    let plan = app.session.execute("gmcut.inspect", json!({})).map_err(|error| error.to_string())?;
    if plan["fresh"].as_bool() != Some(true) {
        return Err("현재 시퀀스를 먼저 분석하세요".into());
    }
    let index = params
        .get("index")
        .or_else(|| plan.get("cursor"))
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or("잘못된 후보 번호입니다")?;
    let candidate = plan["candidates"].as_array().and_then(|values| values.get(index)).ok_or("현재 후보가 없습니다")?;
    let start = candidate["pauseStart"].as_i64().or_else(|| candidate["start"].as_i64()).ok_or("후보 시작 시간이 없습니다")?;
    let end = candidate["pauseEnd"].as_i64().or_else(|| candidate["end"].as_i64()).ok_or("후보 종료 시간이 없습니다")?;
    let context = Tick::from_seconds_f64(0.8);
    let start = Tick(start.saturating_sub(context.0)).max(Tick::ZERO);
    let duration = app.session.active_sequence().ok_or("열린 시퀀스가 없습니다")?.duration();
    let end = (Tick(end) + Tick::from_seconds_f64(0.7)).min(duration);
    app.session.set_playhead(start);
    app.play(1.0);
    app.playback.stop_at = Some(end);
    app.playback.rest_at = Some(start);
    Ok(json!({"start":start.0,"end":end.0}))
}

fn shortcut_scope(app: &FilmcraftApp) -> bool {
    let shown = app.ui.dock.is_visible(PanelKind::Gmcut)
        || app.ui.dock.is_visible(PanelKind::GmcutReview)
        || app.ui.floating.iter().any(|group| group.panels.get(group.active).is_some_and(|panel| matches!(panel, PanelKind::Gmcut | PanelKind::GmcutReview)));
    let scope = matches!(app.ui.focused, PanelKind::Gmcut | PanelKind::GmcutReview)
        || app.ui.workspace == "GM컷" && matches!(app.ui.focused, PanelKind::Program | PanelKind::Timeline);
    shown
        && scope
        && app.ui.mode == crate::state::Mode::Edit
        && app.dialog.is_none()
        && app.native_dialog.is_none()
        && app.ui.settings.is_none()
        && app.ui.clip_dialog.is_none()
        && app.ui.extras.dialog.is_none()
}

pub fn reviewing(app: &FilmcraftApp) -> bool {
    shortcut_scope(app) && app.session.gmcut.as_ref().is_some_and(|plan| filmcraft_engine::gmcut::fresh(&app.session, plan) && !plan.running)
}

pub fn undo_available(app: &FilmcraftApp) -> bool {
    reviewing(app) && filmcraft_engine::gmcut::can_undo(&app.session)
}

pub fn handle_shortcuts(app: &mut FilmcraftApp, ctx: &egui::Context) -> bool {
    if !shortcut_scope(app)
        || ctx.text_edit_focused()
        || app.divider_focused(ctx) && ctx.input(|input| input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::ArrowRight))
    {
        return false;
    }
    let plan = update(app, ctx);
    let ready = plan["fresh"].as_bool() == Some(true) && plan["running"].as_bool() != Some(true) && configured(app, &plan);
    let cursor = plan["cursor"].as_u64().unwrap_or(0);
    let count = plan["count"].as_u64().unwrap_or(0);
    let mut action = None;
    let mut consumed = false;
    ctx.input_mut(|input| {
        let undo = input.events.iter().any(|event| matches!(event, egui::Event::Key {key:egui::Key::Z,pressed:true,modifiers,repeat:false,..} if (modifiers.command || modifiers.ctrl) && !modifiers.shift && !modifiers.alt));
        if undo && reviewing(app) {
            input.events.retain(|event| !matches!(event, egui::Event::Key {key:egui::Key::Z,pressed:true,..}));
            if undo_available(app) { action = Some(("gmcut.undo",json!({}))); }
            consumed = true; return;
        }
        for (key, command, params, enabled) in [
            (egui::Key::Num1,"gmcut.choose",json!({"preset":1}),ready && cursor < count),
            (egui::Key::Num2,"gmcut.choose",json!({"preset":2}),ready && cursor < count),
            (egui::Key::Num3,"gmcut.choose",json!({"preset":3}),ready && cursor < count),
            (egui::Key::P,"playback.gmcutPreview",json!({"index":if cursor < count {cursor} else {0}}),ready && count > 0),
            (egui::Key::ArrowLeft,"gmcut.move",json!({"index":cursor.saturating_sub(1)}),ready && cursor > 0),
            (egui::Key::ArrowRight,"gmcut.move",json!({"index":cursor.saturating_add(1)}),ready && cursor.saturating_add(1) < count),
            (egui::Key::S,"gmcut.skip",json!({}),ready && cursor < count),
            (egui::Key::Escape,"playback.stop",json!({}),true),
        ] {
            let unmodified = input.events.iter().any(|event| matches!(event,egui::Event::Key {key:pressed,pressed:true,modifiers,repeat:false,..} if *pressed == key && *modifiers == egui::Modifiers::NONE));
            if unmodified && input.consume_key(egui::Modifiers::NONE,key) { consumed = true; if enabled { action=Some((command,params)); } break; }
        }
    });
    if let Some((command, params)) = action {
        run(app, ctx, command, params);
    }
    consumed
}
