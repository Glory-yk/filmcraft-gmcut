//! Native GM컷: read-only asynchronous waveform analysis and one explicit undoable apply.
use crate::commands::{CommandSpec, always, bad, has_seq};
use crate::{EngineError, Result, Session};
use filmcraft_edit::transcript as tx;
use filmcraft_project::{ItemId, Project, Sequence};
use filmcraft_speech::GmCutCandidate;
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, atomic::Ordering};

#[derive(Clone)]
pub struct Candidate {
    pub start: Tick,
    pub end: Tick,
    pub pause_start: Tick,
    pub pause_end: Tick,
    pub whole_only: bool,
    keep: Option<Tick>,
    skipped: bool,
}
struct Change {
    cursor: usize,
    choices: Vec<(usize, Option<Tick>, bool)>,
}
type Output = std::result::Result<Vec<GmCutCandidate>, String>;
pub struct Preview {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
}
pub struct Plan {
    project: Arc<Project>,
    path: Option<String>,
    sequence: ItemId,
    revision: u64,
    rate: FrameRate,
    duration: Tick,
    pub candidates: Vec<Candidate>,
    pub cursor: usize,
    min: Tick,
    keep: Tick,
    presets: [Tick; 3],
    sensitivity: String,
    history: Vec<Change>,
    output: Arc<Mutex<Option<Output>>>,
    pub job: u64,
    pub running: bool,
    complete: bool,
    pub status: String,
    pub preview: Option<Arc<Preview>>,
    pub preview_windows: Vec<TimeRange>,
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
pub fn fresh(s: &Session, p: &Plan) -> bool {
    s.revision == p.revision && s.path == p.path && s.state.active_sequence == Some(p.sequence) && Arc::ptr_eq(&s.project, &p.project)
}
pub fn can_undo(s: &Session) -> bool {
    s.gmcut.as_ref().is_some_and(|p| fresh(s, p) && !p.running && p.complete && !p.history.is_empty())
}
fn ready(s: &Session) -> Result<&Plan> {
    s.gmcut
        .as_ref()
        .filter(|p| fresh(s, p) && !p.running && p.complete)
        .ok_or_else(|| bad("gmcut", "무음 분석이 없거나 편집 상태가 바뀌었습니다. 다시 분석해 주세요."))
}
fn seconds(t: Tick) -> f64 {
    t.seconds()
}
fn number(p: &Value, key: &str, default: f64, max: f64) -> Result<Tick> {
    let v = p.get(key).map_or(Some(default), Value::as_f64).ok_or_else(|| bad("gmcut", format!("{key} must be seconds")))?;
    if !v.is_finite() || !(0.0..=max).contains(&v) {
        return Err(bad("gmcut", format!("{key} is out of range")));
    }
    Ok(Tick::from_seconds_f64(v))
}

/// Exact integer equivalent of standalone core.mjs::pauseCut (positive half ties round up).
fn cut(c: &Candidate, keep: Tick, rate: FrameRate) -> Option<TimeRange> {
    if keep < Tick::ZERO || rate.num <= 0 || rate.den <= 0 || (keep > Tick::ZERO && c.whole_only) {
        return None;
    }
    let (start, end) = if keep == Tick::ZERO { (c.pause_start, c.pause_end) } else { (c.start, c.end) };
    let d = i128::from(TICKS_PER_SECOND) * i128::from(rate.den);
    let n = i128::from(rate.num);
    let ceil = |t: Tick| {
        let x = i128::from(t.0) * n;
        x.div_euclid(d) + i128::from(x.rem_euclid(d) != 0)
    };
    let lo = ceil(start);
    let hi = (i128::from(end.0) * n).div_euclid(d);
    let desired = i128::from((c.pause_end - c.pause_start - keep).0.max(0)) * n;
    let frames = ((desired * 2 + d).div_euclid(d * 2)).min(hi - lo).max(0);
    if frames <= 0 {
        return None;
    }
    let center = (i128::from(start.0) + i128::from(end.0)) * n - frames * d;
    let first = (center + d).div_euclid(d * 2).max(lo).min(hi - frames);
    let a = i64::try_from(first).ok()?;
    let b = i64::try_from(first + frames).ok()?;
    let range = TimeRange::from_bounds(rate.tick_of(a), rate.tick_of(b));
    (range.duration > Tick::ZERO).then_some(range)
}

fn supported(s: &Session) -> Result<(ItemId, Tick)> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if q.settings.frame_rate.num <= 0
        || q.settings.frame_rate.den <= 0
        || !(1.0..=240.0).contains(&q.settings.frame_rate.as_f64())
        || !(8000..=192000).contains(&q.settings.sample_rate)
    {
        return Err(bad("gmcut.analyze", "invalid sequence frame/sample rate"));
    }
    let audios: Vec<_> = q.audio_tracks.iter().flat_map(|t| t.items.iter().map(move |c| (t, c))).collect();
    let videos: Vec<_> = q.video_tracks.iter().flat_map(|t| t.items.iter().map(move |c| (t, c))).collect();
    let Some((track, audio)) = audios.first().copied().filter(|_| audios.len() == 1) else {
        return Err(bad("gmcut.analyze", "첫 버전은 시작 0의 단일 음성 클립 또는 연결 A/V만 지원합니다."));
    };
    let plain = |c: &filmcraft_project::TrackItem| {
        c.enabled && c.start == Tick::ZERO && c.source_in == Tick::ZERO && c.speed == 1.0 && !c.reverse && c.frame_hold.is_none() && c.multicam.is_none()
    };
    if !track.enabled || track.muted || !plain(audio) || videos.len() > 1 {
        return Err(bad("gmcut.analyze", "다중 클립·음소거·트림·속도 변경·중첩은 지원하지 않습니다."));
    }
    if let Some((track, video)) = videos.first().copied()
        && (!track.enabled || !plain(video) || video.item != audio.item || video.duration != audio.duration || video.link.is_none() || video.link != audio.link)
    {
        return Err(bad("gmcut.analyze", "동일한 원본·시작·길이의 연결 영상/음성이 필요합니다."));
    }
    let media =
        s.project.item(audio.item).and_then(|i| i.as_media()).ok_or_else(|| bad("gmcut.analyze", "중첩/하위 클립 대신 원본 미디어를 사용해 주세요."))?;
    if media.offline || media.info.audio_streams.is_empty() || media.info.video.as_ref().is_some_and(|v| v.frame_rate != q.settings.frame_rate) {
        return Err(bad("gmcut.analyze", "온라인 음성 및 원본과 같은 시퀀스 프레임률이 필요합니다."));
    }
    let duration = q.duration();
    if duration <= Tick::ZERO || duration > Tick::from_units(3600, 1) || duration != audio.duration {
        return Err(bad("gmcut.analyze", "1시간 이하의 단일 클립 시퀀스가 필요합니다."));
    }
    Ok((audio.item, duration))
}

fn analysis_mono(channels: &[Vec<f32>]) -> std::result::Result<Vec<f32>, String> {
    match channels {
        [mono] => Ok(mono.clone()),
        [left, right] if left.len() == right.len() => {
            // Match the analysis reference's equal-power stereo downmix; ASR keeps its own mix.
            Ok(left.iter().zip(right).map(|(l, r)| (l + r) * std::f32::consts::FRAC_1_SQRT_2).collect())
        }
        [_, _] => Err("GM컷 음성 채널 길이가 일치하지 않습니다.".into()),
        _ => Err("GM컷 분석은 모노 또는 스테레오 음성만 지원합니다.".into()),
    }
}

fn analyze(s: &mut Session, p: &Value) -> Result<Value> {
    poll(s);
    if s.gmcut.as_ref().is_some_and(|a| a.running) {
        return Err(bad("gmcut.analyze", "분석 중입니다. 취소한 뒤 다시 실행해 주세요."));
    }
    let min = number(p, "minSeconds", 0.05, 30.)?;
    if min < Tick::from_units(50, 1000) {
        return Err(bad("gmcut.analyze", "minSeconds must be at least 0.05"));
    }
    let keep = number(p, "keepSeconds", 0., 10.)?;
    let sensitivity = p
        .get("sensitivity")
        .map_or(Some("normal"), Value::as_str)
        .filter(|v| matches!(*v, "quiet" | "normal" | "sensitive"))
        .ok_or_else(|| bad("gmcut.analyze", "invalid sensitivity"))?
        .to_string();
    let mut presets = [Tick::ZERO, Tick::from_units(2, 10), Tick::from_units(3, 10)];
    if let Some(v) = p.get("presets") {
        let values = v.as_array().filter(|v| v.len() == 3).ok_or_else(|| bad("gmcut.analyze", "exactly three presets are required"))?;
        for (out, value) in presets.iter_mut().zip(values) {
            *out = number(&json!({"preset":value}), "preset", 0., 10.)?;
        }
    }
    let (_, duration) = supported(s)?;
    let sequence = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let rate = s.sequence_rate();
    let sr = s.active_sequence().ok_or(EngineError::NoSequence)?.settings.sample_rate;
    let transcriber =
        s.gmcut_analyzer.clone().or_else(|| s.transcriber.clone()).ok_or_else(|| bad("gmcut.analyze", "로컬 GM컷 분석기가 연결되지 않았습니다."))?;
    let project = s.project.clone();
    let provider = s.media.full_res_provider(project.clone(), s.services.clone());
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0).checked_add(1).ok_or_else(|| bad("gmcut.analyze", "job id overflow"))?;
    let job = crate::Job { id, label: "GM컷 무음 분석".into(), progress: Default::default(), result: Default::default() };
    job.progress.total.store(1000, Ordering::Relaxed);
    let progress = job.progress.clone();
    let output = Arc::new(Mutex::new(None));
    let out = output.clone();
    let sensitivity_worker = sensitivity.clone();
    std::thread::Builder::new()
        .name("filmcraft-gmcut".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Output {
                let q = project.sequence(sequence).ok_or("sequence disappeared")?;
                let frames = usize::try_from(duration.to_units_floor(16000)).map_err(|_| "invalid PCM duration")?;
                if frames == 0 || frames > 16_000 * 3600 {
                    return Err("invalid PCM frame count".into());
                }
                *lock(&progress.status) = "원본 음성 읽는 중…".into();
                let mut resampler = filmcraft_audio_dsp::resample::StreamResampler::new(sr, 16000);
                let mut cancelled = false;
                let channels = resampler.process(0, frames, |at, n| {
                    if progress.cancel.load(Ordering::Relaxed) {
                        cancelled = true;
                        return Vec::new();
                    }
                    filmcraft_render::audio::mix_sequence(&project, q, at, n, &provider).channels
                });
                if cancelled || progress.cancel.load(Ordering::Relaxed) {
                    return Err("분석 취소 · 변경 없음".into());
                }
                let mono = analysis_mono(&channels)?;
                if mono.is_empty() || mono.iter().any(|v| !v.is_finite()) {
                    return Err("유효한 PCM 음성이 없습니다.".into());
                }
                transcriber
                    .gmcut_candidates(&mono, seconds(min), &sensitivity_worker, rate, &mut |fraction, status| {
                        if fraction.is_finite() {
                            progress.done.store((fraction.clamp(0., 1.) * 1000.) as u64, Ordering::Relaxed);
                        }
                        *lock(&progress.status) = status.to_string();
                        !progress.cancel.load(Ordering::Relaxed)
                    })
                    .map_err(|e| e.to_string())
            }))
            .unwrap_or_else(|_| Err("GM컷 분석 작업 실패 · 변경 없음".into()));
            *lock(&out) = Some(result);
        })
        .map_err(|e| EngineError::Other(format!("Cannot start GM컷 worker: {e}")))?;
    s.gmcut = Some(Plan {
        project: s.project.clone(),
        path: s.path.clone(),
        sequence,
        revision: s.revision,
        rate,
        duration,
        candidates: Vec::new(),
        cursor: 0,
        min,
        keep,
        presets,
        sensitivity,
        history: Vec::new(),
        output,
        job: id,
        running: true,
        complete: false,
        status: "무음 분석 중…".into(),
        preview: None,
        preview_windows: Vec::new(),
    });
    s.jobs.push(job);
    Ok(json!({"job":id,"changed":false,"running":true}))
}

fn validate(values: Vec<GmCutCandidate>, duration: Tick, min: Tick) -> std::result::Result<Vec<Candidate>, String> {
    if values.len() > 5000 {
        return Err("too many GM컷 candidates".into());
    }
    let mut out = Vec::with_capacity(values.len());
    let mut previous = Tick::ZERO;
    for c in values {
        if c.pause_start < Tick::ZERO
            || c.start < c.pause_start
            || c.end <= c.start
            || c.end > c.pause_end
            || c.pause_end > duration
            || c.pause_end - c.pause_start < min
            || c.start < previous
        {
            return Err("invalid GM컷 candidate bounds/order".into());
        }
        previous = c.end;
        out.push(Candidate {
            start: c.start,
            end: c.end,
            pause_start: c.pause_start,
            pause_end: c.pause_end,
            whole_only: c.whole_only,
            keep: None,
            skipped: false,
        });
    }
    Ok(out)
}
pub fn poll(s: &mut Session) {
    let Some(mut a) = s.gmcut.take() else { return };
    if a.running {
        let stale = !fresh(s, &a);
        let cancelled = s.jobs.iter().find(|j| j.id == a.job).is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
        if stale && let Some(job) = s.jobs.iter().find(|j| j.id == a.job) {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        let result = lock(&a.output).take();
        if let Some(result) = result {
            let result = if stale {
                Err("타임라인 변경 · 분석 결과 폐기".into())
            } else if cancelled {
                Err("분석 취소 · 변경 없음".into())
            } else {
                result.and_then(|values| validate(values, a.duration, a.min))
            };
            a.running = false;
            match result {
                Ok(values) => {
                    a.candidates = values;
                    a.complete = true;
                    a.status = format!("무음 후보 {}개 · 검토 후 타임라인에 반영하세요.", a.candidates.len());
                }
                Err(error) => {
                    a.complete = false;
                    a.status = error;
                }
            }
            if let Some(job) = s.jobs.iter().find(|j| j.id == a.job) {
                *lock(&job.progress.status) = a.status.clone();
                *lock(&job.result) = Some(if a.complete {
                    Ok(filmcraft_export::Report { path: String::new(), frames: 0, seconds: 0., bytes: 0, render_fps: 0., extra_files: Vec::new() })
                } else {
                    Err(a.status.clone())
                });
                job.progress.done.store(1000, Ordering::Relaxed);
                job.progress.finished.store(true, Ordering::Relaxed);
            }
        }
    }
    s.gmcut = Some(a);
}
fn clear_preview(a: &mut Plan) {
    a.preview = None;
    a.preview_windows.clear();
}
fn inspect(s: &mut Session, _: &Value) -> Result<Value> {
    poll(s);
    let Some(a) = &s.gmcut else {
        return Ok(json!({"fresh":false,"running":false,"count":0,"cursor":0,"candidates":[],"reviewed":0}));
    };
    let candidates: Vec<_> = a.candidates.iter().enumerate().map(|(id, c)| {
        let r = c.keep.and_then(|k| cut(c, k, a.rate));
        json!({"id":id,"start":c.start.0,"end":c.end.0,"pauseStart":c.pause_start.0,"pauseEnd":c.pause_end.0,"wholeOnly":c.whole_only,"keepSeconds":c.keep.map(seconds),"skipped":c.skipped,
            "actualKeepSeconds":seconds(c.pause_end-c.pause_start-r.map(|r|r.duration).unwrap_or_default()),"cut":r.map(|r|json!({"start":r.start.0,"end":r.end().0}))})
    }).collect();
    let ranges = tx::merge_ranges(a.candidates.iter().filter_map(|c| c.keep.and_then(|k| cut(c, k, a.rate))).collect());
    let removed = ranges.iter().fold(Tick::ZERO, |sum, r| sum + r.duration);
    let windows = a.preview_windows.first().zip(a.preview_windows.get(1));
    Ok(
        json!({"fresh":fresh(s,a)&&a.complete&&!a.running,"running":a.running,"status":a.status,"job":a.job,"cursor":a.cursor,"count":candidates.len(),"candidates":candidates,
        "reviewed":a.candidates.iter().filter(|c|c.keep.is_some()||c.skipped).count(),"minSeconds":seconds(a.min),"keepSeconds":seconds(a.keep),"presets":a.presets.map(seconds),"sensitivity":a.sensitivity,
        "source":"gmcut-two-pass-vad-rms","removeSeconds":seconds(removed),"preview":a.preview.as_ref().map(|p|json!({"sampleRate":p.sample_rate,"frames":p.channels.first().map(Vec::len).unwrap_or(0),
            "leftStart":windows.map(|(l,_)|l.start.0),"leftEnd":windows.map(|(l,_)|l.end().0),"rightStart":windows.map(|(_,r)|r.start.0),"rightEnd":windows.map(|(_,r)|r.end().0)}))}),
    )
}

fn change(s: &mut Session, p: &Value, automatic: bool, skip: bool) -> Result<Value> {
    poll(s);
    let a = ready(s)?;
    let keep = if automatic {
        number(p, "keepSeconds", seconds(a.keep), 10.)?
    } else if skip {
        Tick::ZERO
    } else {
        let index = p.get("preset").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).and_then(|n| n.checked_sub(1));
        *index.and_then(|n| a.presets.get(n)).ok_or_else(|| bad("gmcut.choose", "preset must be 1, 2 or 3"))?
    };
    if (!automatic && a.cursor >= a.candidates.len()) || a.candidates.is_empty() {
        return Err(bad("gmcut.choose", "검토할 후보가 없습니다."));
    }
    let a = s.gmcut.as_mut().ok_or_else(|| bad("gmcut", "no plan"))?;
    let indices: Vec<_> = if automatic { (0..a.candidates.len()).collect() } else { vec![a.cursor] };
    let choices = indices.iter().filter_map(|i| a.candidates.get(*i).map(|c| (*i, c.keep, c.skipped))).collect();
    a.history.push(Change { cursor: a.cursor, choices });
    if a.history.len() > 100 {
        a.history.remove(0);
    }
    for i in indices {
        if let Some(c) = a.candidates.get_mut(i) {
            c.keep = (!skip).then_some(keep);
            c.skipped = skip;
        }
    }
    a.cursor = if automatic { a.candidates.len() } else { a.cursor.saturating_add(1) };
    clear_preview(a);
    let next = a.candidates.get(a.cursor).map(|c| c.pause_start);
    if let Some(next) = next {
        s.set_playhead(next);
    }
    inspect(s, &json!({}))
}
fn move_to(s: &mut Session, p: &Value) -> Result<Value> {
    poll(s);
    let a = ready(s)?;
    let index = p
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|i| *i < a.candidates.len())
        .ok_or_else(|| bad("gmcut.move", "invalid candidate index"))?;
    let start = a.candidates.get(index).ok_or_else(|| bad("gmcut.move", "missing candidate"))?.pause_start;
    if let Some(a) = s.gmcut.as_mut() {
        a.cursor = index;
        clear_preview(a);
    }
    s.set_playhead(start);
    inspect(s, &json!({}))
}
fn undo(s: &mut Session, _: &Value) -> Result<Value> {
    poll(s);
    ready(s)?;
    let a = s.gmcut.as_mut().ok_or_else(|| bad("gmcut.undo", "no plan"))?;
    let old = a.history.pop().ok_or_else(|| bad("gmcut.undo", "실행취소할 검토가 없습니다."))?;
    for (i, keep, skipped) in old.choices {
        if let Some(c) = a.candidates.get_mut(i) {
            c.keep = keep;
            c.skipped = skipped;
        }
    }
    a.cursor = old.cursor;
    clear_preview(a);
    let start = a.candidates.get(a.cursor).map(|c| c.pause_start);
    if let Some(start) = start {
        s.set_playhead(start);
    }
    inspect(s, &json!({}))
}
fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    poll(s);
    let a = ready(s)?;
    let index = match p.get("index") {
        None => a.cursor,
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad("gmcut.preview", "invalid candidate index"))?,
    };
    let c = a.candidates.get(index).ok_or_else(|| bad("gmcut.preview", "missing candidate"))?;
    let r = cut(c, c.keep.unwrap_or(a.keep), a.rate).ok_or_else(|| bad("gmcut.preview", "이 후보는 현재 유지 설정에서 보존됩니다."))?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let sr = q.settings.sample_rate;
    if !(8000..=192000).contains(&sr) {
        return Err(bad("gmcut.preview", "invalid sample rate"));
    }
    let context = Tick::from_units(8, 10);
    let left = TimeRange::from_bounds((r.start - context).max(Tick::ZERO), r.start);
    let right = TimeRange::from_bounds(r.end(), (r.end() + context).min(q.duration()));
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let mut channels: Vec<Vec<f32>> = Vec::new();
    for range in [left, right] {
        let count = usize::try_from(range.duration.to_units_floor(i64::from(sr))).map_err(|_| bad("gmcut.preview", "invalid preview duration"))?;
        let audio = filmcraft_render::audio::mix_sequence(&s.project, q, range.start.to_units_floor(i64::from(sr)), count, &provider);
        if audio.channels.len() > 8 || audio.channels.iter().flatten().any(|x| !x.is_finite()) {
            return Err(bad("gmcut.preview", "invalid preview audio"));
        }
        if channels.is_empty() {
            channels.resize_with(audio.channels.len(), Vec::new);
        }
        if channels.len() != audio.channels.len() {
            return Err(bad("gmcut.preview", "preview channel count changed"));
        }
        for (to, from) in channels.iter_mut().zip(audio.channels) {
            to.extend(from);
        }
    }
    let frames = channels.first().map(Vec::len).unwrap_or(0);
    if frames == 0 {
        return Err(bad("gmcut.preview", "no preview audio"));
    }
    if let Some(a) = s.gmcut.as_mut() {
        a.preview = Some(Arc::new(Preview { channels, sample_rate: sr }));
        a.preview_windows = vec![left, right];
    }
    Ok(
        json!({"index":index,"frames":frames,"sampleRate":sr,"start":r.start.0,"end":r.end().0,"leftStart":left.start.0,"leftEnd":left.end().0,"rightStart":right.start.0,"rightEnd":right.end().0}),
    )
}
fn apply(s: &mut Session, _: &Value) -> Result<Value> {
    poll(s);
    let a = ready(s)?;
    supported(s)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if q.all_tracks().any(|t| t.locked) || q.caption_tracks.iter().any(|t| t.locked || !t.sync_lock) {
        return Err(bad("gmcut.apply", "영상·음성 잠금을 해제하고 모든 자막 동기 잠금을 켜 주세요."));
    }
    let ranges = tx::merge_ranges(a.candidates.iter().filter_map(|c| c.keep.and_then(|k| cut(c, k, a.rate))).collect());
    if ranges.is_empty() {
        return Err(bad("gmcut.apply", "반영할 컷이 없습니다."));
    }
    if ranges.iter().fold(Tick::ZERO, |sum, r| sum + r.duration) >= q.duration() {
        return Err(bad("gmcut.apply", "최소 한 구간을 보존해야 합니다. 모든 원본을 삭제하지 않았습니다."));
    }
    let words = crate::transcript::sequence_words(s);
    if words.iter().any(|w| ranges.iter().any(|r| w.start < r.end() && r.start < w.end)) {
        return Err(bad("gmcut.apply", "컷이 ASR 단어 구간을 가로지릅니다. 경계를 확인하거나 해당 후보를 보존해 주세요."));
    }
    if q.caption_tracks.iter().flat_map(|t| &t.captions).any(|c| !c.text.trim().is_empty() && ranges.iter().any(|r| r.start <= c.start && c.end() <= r.end())) {
        return Err(bad("gmcut.apply", "삭제 구간 안에 자막이 있습니다. 자막/음성 경계를 확인해 주세요."));
    }
    let expected_words: Vec<_> = words.into_iter().map(|w| w.text).collect();
    let sources = s.project.transcripts.clone();
    let count = ranges.len();
    let reported: Vec<_> = ranges.iter().map(|r| json!({"start":r.start.0,"end":r.end().0})).collect();
    let total = s.edit_sequence("GM컷", move |q, ctx, state| {
        let audio_ticks = |q: &Sequence| q.audio_tracks.iter().flat_map(|t| &t.items).filter(|c| c.enabled).map(|c| i128::from(c.duration.0)).sum::<i128>();
        let before = audio_ticks(q);
        let expected: i128 = ranges.iter().map(|r| i128::from(r.duration.0)).sum();
        let total = tx::ripple_delete_ranges(q, ranges.clone(), ctx);
        if before - audio_ticks(q) != expected {
            return Err(bad("gmcut.apply", "음성 컷 검증 실패 · 전체 변경 취소"));
        }
        if tx::sequence_words(q, &sources).into_iter().map(|w| w.text).collect::<Vec<_>>() != expected_words {
            return Err(bad("gmcut.apply", "단어 보존 검증 실패 · 전체 변경 취소"));
        }
        if state.ripple_sequence_markers {
            let map = |mut t: Tick| {
                for r in ranges.iter().rev() {
                    if t >= r.end() {
                        t -= r.duration;
                    } else if t >= r.start {
                        t = r.start;
                    }
                }
                t
            };
            for marker in &mut q.markers {
                let end = map(marker.start + marker.duration);
                marker.start = map(marker.start);
                marker.duration = (end - marker.start).max(Tick::ZERO);
            }
        }
        state.caption_selection.clear();
        Ok(total)
    })?;
    if let Some(a) = s.gmcut.as_mut() {
        clear_preview(a);
        a.status = "타임라인 반영 완료 · 편집 실행취소로 복원할 수 있습니다.".into();
    }
    Ok(json!({"removed":count,"ticks":total.0,"seconds":seconds(total),"ranges":reported,"undo":"edit.undo"}))
}
fn reset(s: &mut Session, _: &Value) -> Result<Value> {
    if let Some(a) = s.gmcut.take()
        && a.running
        && let Some(job) = s.jobs.iter().find(|j| j.id == a.job)
    {
        job.progress.cancel.store(true, Ordering::Relaxed);
        job.progress.finished.store(true, Ordering::Relaxed);
        *lock(&job.result) = Some(Err("GM컷 분석 취소 · 변경 없음".into()));
    }
    Ok(json!({"changed":false}))
}
pub fn commands() -> Vec<CommandSpec> {
    type Run = fn(&mut Session, &Value) -> Result<Value>;
    let entries: [(&str, &str, &str, Run); 10] = [
        ("gmcut.analyze", "GM컷", "{minSeconds?:f,keepSeconds?:f,presets?:[f,f,f],sensitivity?:str}", analyze),
        ("gmcut.inspect", "Inspect GM컷", "{}", inspect),
        ("gmcut.choose", "Choose GM컷 preset", "{preset:1|2|3}", |s, p| change(s, p, false, false)),
        ("gmcut.skip", "Skip GM컷 pause", "{}", |s, p| change(s, p, false, true)),
        ("gmcut.move", "Move GM컷 cursor", "{index:n}", move_to),
        ("gmcut.undo", "Undo GM컷 review", "{}", undo),
        ("gmcut.auto", "Automatic GM컷", "{keepSeconds?:f}", |s, p| change(s, p, true, false)),
        ("gmcut.preview", "Preview GM컷", "{index?:n}", preview),
        ("gmcut.apply", "Apply GM컷", "{}", apply),
        ("gmcut.reset", "Reset GM컷 review", "{}", reset),
    ];
    entries
        .into_iter()
        .map(|(id, label, params, run)| CommandSpec {
            id,
            label,
            menu: if id == "gmcut.analyze" { &["Sequence", "Transcript"] } else { &[] },
            shortcut: None,
            params,
            enabled: if matches!(id, "gmcut.inspect" | "gmcut.reset") { always } else { has_seq },
            run,
            journal: id == "gmcut.apply",
        })
        .collect()
}
#[cfg(test)]
mod tests;
