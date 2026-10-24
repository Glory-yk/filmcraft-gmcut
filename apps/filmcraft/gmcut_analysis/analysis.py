"""Standalone analysis and non-destructive source-time cut plans."""
import math
import os
import subprocess
from pathlib import Path

MODEL = Path(os.environ.get('GMCUT_MODEL') or Path(os.environ.get('XDG_CACHE_HOME') or Path.home() / '.cache') / 'gmcut' / 'silero_vad.jit').expanduser()


def number(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(f'{name} must be a finite number')
    return float(value)


def normalize_cuts(cuts, duration, fps):
    """Snap removal inward; never remove a sample outside the approved range."""
    duration, fps = number(duration, 'duration'), number(fps, 'fps')
    if duration <= 0 or fps <= 0:
        raise ValueError('Invalid media duration or frame rate')
    if not isinstance(cuts, list) or len(cuts) > 5000:
        raise ValueError('cuts must be an array of at most 5000 ranges')
    ranges = []
    for cut in cuts:
        if not isinstance(cut, dict):
            raise ValueError('Each cut must contain start and end')
        a, b = number(cut.get('start'), 'start'), number(cut.get('end'), 'end')
        if a < 0 or b > duration + 1e-7 or a >= b:
            raise ValueError('Cut is outside the source or has no duration')
        ranges.append((a, min(b, duration)))
    ranges.sort()
    for left, right in zip(ranges, ranges[1:]):
        if right[0] < left[1] - 1e-9:
            raise ValueError('Cut ranges overlap')
    snapped = []
    for a, b in ranges:
        a = math.ceil(a * fps - 1e-8) / fps
        b = math.floor(b * fps + 1e-8) / fps
        if b > a:
            snapped.append({'start': a, 'end': b})
    return snapped


def make_manifest(source, cuts, decisions=None):
    cuts = normalize_cuts(cuts, source['duration'], source['fps'])
    segments, cursor, destination = [], 0.0, 0.0
    for cut in cuts + [{'start': source['duration'], 'end': source['duration']}]:
        if cut['start'] > cursor + 1e-9:
            length = cut['start'] - cursor
            segments.append({'sourceStart': cursor, 'sourceEnd': cut['start'],
                             'timelineStart': destination, 'timelineEnd': destination + length})
            destination += length
        cursor = cut['end']
    if not segments:
        raise ValueError('At least one source segment must remain')
    if decisions is not None and (not isinstance(decisions, list) or len(decisions) > 5000):
        raise ValueError('decisions must be an array of at most 5000 entries')
    return {'schema': 'gencut.cut-plan.v1', 'source': source.copy(),
            'cuts': cuts, 'segments': segments, 'duration': destination,
            'removedSeconds': source['duration'] - destination, 'decisions': decisions or []}


def pause_candidates(source, db, speech, minimum, threshold):
    """Find full pauses first, then retain the existing speech protection inside them."""
    import numpy as np
    from vendor import silence_analysis as legacy
    protected = legacy.protect_speech(speech, before_ms=40, after_ms=80)
    quiet = db < -38.0
    safe = quiet & ~protected
    raw_pauses = legacy.masks_to_intervals(quiet & ~speech)
    safe_pauses = legacy.masks_to_intervals(safe)
    candidates, pause_count = [], 0
    for a, b in raw_pauses:
        pause_start, pause_end = a / 16000, min(b / 16000, source['duration'])
        if pause_end - pause_start + 1e-8 < minimum:
            continue
        pause_count += 1
        lo, hi = a // 160, b // 160
        has_safe_frame = False
        # Minimum length describes the full pause; guards only narrow what may be cut.
        for x, y in legacy.masks_to_intervals(safe[lo:hi]):
            start, end = (a + x) / 16000, min((a + y) / 16000, source['duration'])
            if end <= start or not normalize_cuts([{'start': start, 'end': end}], source['duration'], source['fps']):
                continue
            has_safe_frame = True
            if np.percentile(db[(a + x) // 160:(a + y + 159) // 160], 90) >= threshold:
                continue
            candidates.append({'id': f"{source['id']}:{len(candidates)}", 'start': start,
                               'end': end, 'duration': end - start,
                               'pauseStart': pause_start, 'pauseEnd': pause_end})
        # Whole removal permits raw quiet frames even when artificial guards consume the gap.
        if (not has_safe_frame
                and normalize_cuts([{'start': pause_start, 'end': pause_end}], source['duration'], source['fps'])
                and np.percentile(db[lo:hi], 90) < threshold):
            candidates.append({'id': f"{source['id']}:{len(candidates)}", 'start': pause_start,
                               'end': pause_end, 'duration': pause_end - pause_start,
                               'pauseStart': pause_start, 'pauseEnd': pause_end, 'wholeOnly': True})
    diagnostics = {'pauseCount': pause_count, 'safeCandidateCount': len(candidates),
                   'longestPauseSeconds': max(((b - a) / 16000 for a, b in raw_pauses), default=0),
                   'protectedOnlyMaxSeconds': max(((b - a) / 16000 for a, b in safe_pauses), default=0),
                   'rmsThresholdDbfs': threshold,
                   'wholeOnlyCandidateCount': sum(c.get('wholeOnly', False) for c in candidates)}
    return candidates, diagnostics


def analyze_source(source, work_dir, minimum=0.05, sensitivity='normal', progress=lambda p: None):
    import numpy as np
    minimum = number(minimum, 'minSilence')
    if not 0.05 <= minimum <= 30:
        raise ValueError('minSilence must be between 0.05 and 30 seconds')
    if sensitivity not in ('quiet', 'normal', 'sensitive'):
        raise ValueError('Unknown sensitivity')
    if not source['hasAudio']:
        raise ValueError('This media has no audio track')
    if not MODEL.is_file():
        raise ValueError('Silero model unavailable. Run: python setup_model.py (or set GMCUT_MODEL).')
    raw_path = work_dir / f"{source['id']}.f32"
    progress(5)
    subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-y', '-i', source['path'],
                    '-vn', '-ac', '1', '-ar', '16000', '-f', 'f32le', str(raw_path)],
                   check=True, capture_output=True, timeout=600)
    audio = np.fromfile(raw_path, dtype=np.float32)
    return analyze_pcm(source, audio, minimum, sensitivity, progress)


def analyze_pcm(source, audio, minimum=0.05, sensitivity='normal', progress=lambda p: None):
    """Shared 16 kHz mono PCM analysis without a decoder or source-file writes."""
    import numpy as np
    from vendor import silence_analysis as legacy
    minimum = number(minimum, 'minSilence')
    if not 0.05 <= minimum <= 30:
        raise ValueError('minSilence must be between 0.05 and 30 seconds')
    if sensitivity not in ('quiet', 'normal', 'sensitive'):
        raise ValueError('Unknown sensitivity')
    if not 0 < number(source.get('fps'), 'fps') <= 1000 or not 0 < number(source.get('duration'), 'duration') <= 3600:
        raise ValueError('Invalid PCM source timing')
    audio = np.asarray(audio, dtype=np.float32)
    if audio.ndim != 1 or not 0 < len(audio) <= 16000 * 3600 or not np.isfinite(audio).all():
        raise ValueError('PCM must be finite mono 16 kHz audio of at most one hour')
    if not MODEL.is_file():
        raise ValueError('Silero model unavailable. Run: python setup_model.py (or set GMCUT_MODEL).')
    progress(20)
    _, details = legacy.analyze(audio, MODEL)
    progress(85)
    # All sensitivity levels preserve the same boosted-VAD speech guards.
    db = np.asarray(details['frame_dbfs'])
    probabilities = np.maximum(details['vad_normal'], details['vad_boosted'])
    speech = np.zeros(len(db), dtype=bool)
    for i in range(len(db)):
        lo = (i * legacy.FRAME) // legacy.WINDOW
        hi = min(len(probabilities), ((i + 1) * legacy.FRAME + legacy.WINDOW - 1) // legacy.WINDOW)
        speech[i] = bool(np.any(probabilities[lo:hi] >= legacy.SPEECH_CUTOFF))
    threshold = min(-35.0, details['rms_threshold_dbfs'] + {'quiet': -6, 'normal': 0, 'sensitive': 3}[sensitivity])
    candidates, diagnostics = pause_candidates(source, db, speech, minimum, threshold)
    size = max(1, math.ceil(len(audio) / 8000))
    padded = np.pad(audio, (0, (-len(audio)) % size)).reshape(-1, size)
    waveform = np.stack((padded.min(axis=1), padded.max(axis=1)), axis=1).round(5).tolist()
    progress(100)
    return {'sourceId': source['id'], 'candidates': candidates, 'waveform': waveform,
            'waveInterval': size / 16000, 'method': details['method'],
            'sensitivity': sensitivity, 'rmsThresholdDbfs': threshold,
            'speechGuardBeforeMs': 40, 'speechGuardAfterMs': 80, 'diagnostics': diagnostics}


def render_plan(manifest, output, work_dir):
    source, segments = manifest['source'], manifest['segments']
    if len(segments) > 1000:
        raise ValueError('Render supports at most 1000 retained segments; JSON export supports larger plans')
    filters, joined = [], []
    for i, segment in enumerate(segments):
        a, b = segment['sourceStart'], segment['sourceEnd']
        if source['hasVideo']:
            filters.append(f'[0:v:0]trim=start={a:.9f}:end={b:.9f},setpts=PTS-STARTPTS[v{i}]')
            joined.append(f'[v{i}]')
        if source['hasAudio']:
            filters.append(f'[0:a:0]atrim=start={a:.9f}:end={b:.9f},asetpts=PTS-STARTPTS[a{i}]')
            joined.append(f'[a{i}]')
    v, a = int(source['hasVideo']), int(source['hasAudio'])
    filters.append(''.join(joined) + f'concat=n={len(segments)}:v={v}:a={a}' + ('[v]' if v else '') + ('[a]' if a else ''))
    script = work_dir / (output.stem + '.filters')
    script.write_text(';\n'.join(filters), encoding='utf-8')
    command = ['ffmpeg', '-nostdin', '-v', 'error', '-y', '-i', source['path'], '-/filter_complex', str(script)]
    if v:
        command += ['-map', '[v]', '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '18', '-pix_fmt', 'yuv420p']
    if a:
        command += ['-map', '[a]', '-c:a', 'aac', '-b:a', '192k']
    command += ['-movflags', '+faststart', str(output)]
    subprocess.run(command, check=True, capture_output=True, timeout=3600)
