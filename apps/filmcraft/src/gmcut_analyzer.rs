//! GM컷-only PCM adapter. Normal FilmCraft transcription keeps its original backend.
use filmcraft_project::Transcript;
use filmcraft_speech::{GmCutCandidate, Options, ProgressFn, SAMPLE_RATE, SpeechError, Transcriber, seconds_tick};
use filmcraft_time::{FrameRate, Tick};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct GmCutAnalyzer {
    python: PathBuf,
    script: PathBuf,
    analysis: PathBuf,
    model: Option<PathBuf>,
}

impl GmCutAnalyzer {
    pub fn from_env() -> Result<Self, SpeechError> {
        let executable = std::env::current_exe()?;
        let resources = executable
            .parent()
            .and_then(std::path::Path::parent)
            .ok_or_else(|| SpeechError::Unavailable("FilmCraft Resources directory is unavailable".into()))?
            .join("Resources");
        let python = std::env::var_os("FILMCRAFT_GMCUT_PYTHON")
            .or_else(|| std::env::var_os("FILMCRAFT_LOCAL_VAD_PYTHON"))
            .or_else(|| std::env::var_os("FILMCRAFT_LOCAL_ASR_PYTHON"))
            .map(PathBuf::from)
            .or_else(|| {
                let path = resources.join("gmcut-python.txt");
                fs::metadata(&path)
                    .ok()
                    .filter(|m| m.is_file() && m.len() <= 8192)
                    .and_then(|_| fs::read_to_string(path).ok())
                    .map(|text| PathBuf::from(text.trim()))
            })
            .ok_or_else(|| SpeechError::Unavailable("set FILMCRAFT_GMCUT_PYTHON or package a GM컷 Python runtime pointer".into()))?;
        let script = std::env::var_os("FILMCRAFT_GMCUT_SCRIPT").map(PathBuf::from).unwrap_or_else(|| resources.join("gmcut_pcm.py"));
        let analysis = std::env::var_os("FILMCRAFT_GMCUT_ANALYSIS_DIR").map(PathBuf::from).unwrap_or_else(|| resources.join("gmcut_analysis"));
        let model = std::env::var_os("FILMCRAFT_GMCUT_MODEL")
            .or_else(|| std::env::var_os("FILMCRAFT_LOCAL_VAD_MODEL"))
            .map(PathBuf::from)
            .or_else(|| analysis.join("silero_vad.jit").is_file().then(|| analysis.join("silero_vad.jit")));
        if !python.is_absolute()
            || !python.is_file()
            || !script.is_absolute()
            || !script.is_file()
            || !analysis.is_absolute()
            || !analysis.join("analysis.py").is_file()
            || model.as_ref().is_some_and(|p| !p.is_absolute() || !p.is_file())
        {
            return Err(SpeechError::Unavailable("existing GM컷 Python runtime, Resources worker or shared analyzer is missing".into()));
        }
        Ok(Self { python, script, analysis, model })
    }
}

impl Transcriber for GmCutAnalyzer {
    fn id(&self) -> String {
        "gmcut-shared-pcm".into()
    }
    fn transcribe(&self, _: &[f32], _: &Options, _: ProgressFn) -> Result<Transcript, SpeechError> {
        Err(SpeechError::Unavailable("GM컷 analyzer does not replace FilmCraft transcription".into()))
    }
    fn gmcut_candidates(
        &self,
        audio: &[f32],
        minimum: f64,
        sensitivity: &str,
        rate: FrameRate,
        progress: ProgressFn,
    ) -> Result<Vec<GmCutCandidate>, SpeechError> {
        self.run_gmcut(audio, minimum, sensitivity, rate, progress)
    }
}

const MAX_RESPONSE: u64 = 2 * 1024 * 1024;

fn private_file(path: &std::path::Path) -> Result<fs::File, SpeechError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

struct Scratch {
    directory: PathBuf,
    input: PathBuf,
    output: PathBuf,
    progress: PathBuf,
}
impl Drop for Scratch {
    fn drop(&mut self) {
        for path in [&self.input, &self.output, &self.progress] {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(&self.directory);
    }
}
struct Worker(std::process::Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    start: f64,
    end: f64,
    pause_start: f64,
    pause_end: f64,
    #[serde(default)]
    whole_only: bool,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    candidates: Vec<Candidate>,
    samples: usize,
    sample_rate: u32,
}

fn validate_response(bytes: &[u8], samples: usize, rate: FrameRate) -> Result<Vec<GmCutCandidate>, SpeechError> {
    let response: Response = serde_json::from_slice(bytes).map_err(|e| SpeechError::Io(format!("invalid GM컷 response: {e}")))?;
    if response.samples != samples || response.sample_rate != SAMPLE_RATE || response.candidates.len() > 5000 {
        return Err(SpeechError::Io("GM컷 response size or sample clock mismatch".into()));
    }
    let duration = samples as f64 / f64::from(SAMPLE_RATE);
    let mut previous = Tick::ZERO;
    let mut result = Vec::with_capacity(response.candidates.len());
    for c in response.candidates {
        if [c.start, c.end, c.pause_start, c.pause_end].iter().any(|v| !v.is_finite())
            || c.pause_start < 0.
            || c.start < c.pause_start
            || c.end <= c.start
            || c.pause_end < c.end
            || c.pause_end > duration + 1e-7
        {
            return Err(SpeechError::Io("GM컷 candidate exceeds PCM bounds".into()));
        }
        let candidate = GmCutCandidate {
            start: seconds_tick(c.start),
            end: seconds_tick(c.end),
            pause_start: seconds_tick(c.pause_start),
            pause_end: seconds_tick(c.pause_end),
            whole_only: c.whole_only,
        };
        let frame_start = rate.frame_at(candidate.start);
        let first = if rate.tick_of(frame_start) < candidate.start { frame_start.saturating_add(1) } else { frame_start };
        if candidate.pause_start < previous
            || candidate.start >= candidate.end
            || rate.frame_at(candidate.end) <= first
            || (candidate.whole_only && (candidate.start != candidate.pause_start || candidate.end != candidate.pause_end))
        {
            return Err(SpeechError::Io("GM컷 candidates overlap or have no complete frame".into()));
        }
        previous = candidate.pause_end;
        result.push(candidate);
    }
    Ok(result)
}

impl GmCutAnalyzer {
    fn run_gmcut(&self, audio: &[f32], minimum: f64, sensitivity: &str, rate: FrameRate, progress: ProgressFn) -> Result<Vec<GmCutCandidate>, SpeechError> {
        if audio.is_empty()
            || audio.len() > SAMPLE_RATE as usize * 3600
            || audio.iter().any(|v| !v.is_finite())
            || !minimum.is_finite()
            || !(0.05..=30.).contains(&minimum)
            || !matches!(sensitivity, "quiet" | "normal" | "sensitive")
            || rate.num <= 0
            || rate.den <= 0
            || !(0.001..=1000.).contains(&rate.as_f64())
        {
            return Err(SpeechError::Io("invalid GM컷 PCM or analysis settings".into()));
        }
        if !progress(0., "GM컷 파형 분석 준비") {
            return Err(SpeechError::Cancelled);
        }
        if !self.python.is_file() || !self.script.is_file() || !self.analysis.join("analysis.py").is_file() {
            return Err(SpeechError::Unavailable("existing GM컷 Python runtime or worker is missing".into()));
        }
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| SpeechError::Io(e.to_string()))?.as_nanos();
        let stem = std::env::temp_dir().join(format!("filmcraft-gmcut-{}-{nonce}", std::process::id()));
        let mut directory = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(&stem)?;
        let tmp = Scratch { input: stem.join("audio.f32"), output: stem.join("result.json"), progress: stem.join("progress.json"), directory: stem };
        let mut input = std::io::BufWriter::new(private_file(&tmp.input)?);
        for chunk in audio.chunks(SAMPLE_RATE as usize) {
            if !progress(0.02, "GM컷 PCM 준비") {
                return Err(SpeechError::Cancelled);
            }
            for sample in chunk {
                input.write_all(&sample.to_le_bytes())?;
            }
        }
        input.flush()?;
        drop(input);
        let output = private_file(&tmp.output)?;
        let _ = private_file(&tmp.progress)?;
        let mut command = Command::new(&self.python);
        if let Some(model) = &self.model {
            command.env("FILMCRAFT_GMCUT_MODEL", model);
        }
        let mut child = Worker(
            command
                .arg(&self.script)
                .arg("--audio")
                .arg(&tmp.input)
                .arg("--minimum")
                .arg(minimum.to_string())
                .arg("--sensitivity")
                .arg(sensitivity)
                .arg("--fps")
                .arg(rate.as_f64().to_string())
                .arg("--progress")
                .arg(&tmp.progress)
                .env("FILMCRAFT_GMCUT_ANALYSIS_DIR", &self.analysis)
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .env("HF_HUB_OFFLINE", "1")
                .env("TRANSFORMERS_OFFLINE", "1")
                .env("HF_HUB_DISABLE_TELEMETRY", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::from(output))
                .stderr(Stdio::null())
                .spawn()?,
        );
        let started = Instant::now();
        let status = loop {
            if fs::metadata(&tmp.output)?.len() > MAX_RESPONSE {
                return Err(SpeechError::Io("GM컷 response exceeds 2 MiB".into()));
            }
            if let Some(status) = child.0.try_wait()? {
                break status;
            }
            let fraction = fs::metadata(&tmp.progress)
                .ok()
                .filter(|m| m.len() <= 1024)
                .and_then(|_| fs::read(&tmp.progress).ok())
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v.get("progress").and_then(serde_json::Value::as_f64))
                .filter(|v| v.is_finite())
                .map(|v| (v / 100.).clamp(0., 0.99) as f32)
                .unwrap_or(0.05);
            if started.elapsed() > Duration::from_secs(1800) {
                return Err(SpeechError::Io("GM컷 analysis timed out after 1800 seconds".into()));
            }
            if !progress(fraction, "GM컷 음성 보호 · 무음 분석") {
                return Err(SpeechError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if fs::metadata(&tmp.output)?.len() > MAX_RESPONSE {
            return Err(SpeechError::Io("GM컷 response exceeds 2 MiB".into()));
        }
        let bytes = fs::read(&tmp.output)?;
        if !status.success() {
            return Err(SpeechError::Io("existing local GM컷 analysis failed; check its runtime and model".into()));
        }
        let candidates = validate_response(&bytes, audio.len(), rate)?;
        if !progress(1., "GM컷 후보 준비 완료") {
            return Err(SpeechError::Cancelled);
        }
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_pcm_and_cancellation_never_spawn() {
        let b = GmCutAnalyzer { python: "missing".into(), script: "missing".into(), analysis: "missing".into(), model: None };
        for audio in [vec![], vec![f32::NAN]] {
            assert!(b.run_gmcut(&audio, 0.05, "normal", FrameRate::FPS_30, &mut |_, _| true).is_err());
        }
        assert_eq!(b.run_gmcut(&[0.; 160], 0.05, "normal", FrameRate::FPS_30, &mut |_, _| false), Err(SpeechError::Cancelled));
    }
    #[test]
    fn gmcut_does_not_provide_a_transcription_backend() {
        let b = GmCutAnalyzer { python: "missing".into(), script: "missing".into(), analysis: "missing".into(), model: None };
        assert!(matches!(b.transcribe(&[0.; 160], &Options::default(), &mut |_, _| true), Err(SpeechError::Unavailable(_))));
    }
    #[test]
    fn validates_worker_bounds_and_clock() {
        let payload = br#"{"samples":16000,"sampleRate":16000,"candidates":[{"start":0.2,"end":0.5,"pauseStart":0.1,"pauseEnd":0.6}]}"#;
        assert_eq!(validate_response(payload, 16000, FrameRate::FPS_30).unwrap().len(), 1);
        for payload in [
            br#"{"samples":1,"sampleRate":16000,"candidates":[]}"#.as_slice(),
            br#"{"samples":16000,"sampleRate":16000,"candidates":[{"start":0.2,"end":1.1,"pauseStart":0.1,"pauseEnd":1.2}]}"#,
            br#"{"samples":16000,"sampleRate":16000,"candidates":[{"start":0.2,"end":0.21,"pauseStart":0.1,"pauseEnd":0.3}]}"#,
        ] {
            assert!(validate_response(payload, 16000, FrameRate::FPS_30).is_err());
        }
    }
}
