"""Conservative local speech protection: Silero VAD plus 10 ms energy frames."""
import numpy as np

SAMPLE_RATE = 16000
TICKS_PER_SAMPLE = 15876000
WINDOW = 512
FRAME = 160
SPEECH_CUTOFF = 0.30


def probabilities(audio, model):
    import torch
    model.reset_states()
    result = []
    with torch.inference_mode():
        for start in range(0, len(audio), WINDOW):
            block = audio[start:start + WINDOW]
            if len(block) < WINDOW:
                block = np.pad(block, (0, WINDOW - len(block)))
            result.append(float(model(torch.from_numpy(block.copy()), SAMPLE_RATE).item()))
    return np.asarray(result)


def masks_to_intervals(mask, size=FRAME, minimum_samples=0):
    padded = np.r_[False, mask, False].astype(np.int8)
    starts = np.flatnonzero(np.diff(padded) == 1) * size
    ends = np.flatnonzero(np.diff(padded) == -1) * size
    return [(int(a), int(b)) for a, b in zip(starts, ends) if b - a >= minimum_samples]


def protect_speech(speech, before_ms=40, after_ms=220):
    """Directional protection: Korean sentence endings keep a longer release tail."""
    speech = np.asarray(speech, dtype=bool)
    protected = np.zeros(len(speech), dtype=bool)
    for offset in range(-(before_ms // 10), after_ms // 10 + 1):
        lo, hi = max(0, offset), min(len(speech), len(speech) + offset)
        if hi > lo:
            protected[lo:hi] |= speech[lo - offset:hi - offset]
    return protected


def analyze(audio, model_path):
    import torch
    torch.set_num_threads(1)
    if len(audio) == 0 or len(audio) > SAMPLE_RATE * 3600 or not np.isfinite(audio).all():
        raise ValueError('Invalid analysis audio (finite mono, at most one hour)')
    model = torch.jit.load(str(model_path), map_location='cpu').eval()
    # The boosted pass protects quiet speech even if the original-level VAD misses it.
    normal = probabilities(audio, model)
    boosted = probabilities(np.clip(audio * 4.0, -1.0, 1.0).astype(np.float32), model)
    probs = np.maximum(normal, boosted)
    count = (len(audio) + FRAME - 1) // FRAME
    padded = np.pad(audio, (0, count * FRAME - len(audio)))
    db = 10 * np.log10(np.mean(padded.reshape(-1, FRAME) ** 2, axis=1) + 1e-12)
    floor = float(np.percentile(db[db > -100], 10)) if np.any(db > -100) else -100.0
    threshold = float(np.clip(floor + 16.0, -70.0, -38.0))
    speech = np.zeros(count, dtype=bool)
    for i in range(count):
        lo = (i * FRAME) // WINDOW
        hi = min(len(probs), ((i + 1) * FRAME + WINDOW - 1) // WINDOW)
        speech[i] = bool(np.any(probs[lo:hi] >= SPEECH_CUTOFF))
    # Protect the weak release of a spoken ending longer than the next-word onset.
    protected = protect_speech(speech, before_ms=40, after_ms=220)
    # A brief low click or breath must not split a long pause into sub-threshold fragments.
    # Reject entire candidates whose background level is too high; retain loud transients.
    silent = (db < -38.0) & ~protected
    intervals = [(a, b) for a, b in masks_to_intervals(silent, minimum_samples=1600)
                 if np.percentile(db[a // FRAME:(b + FRAME - 1) // FRAME], 90) < threshold]
    ranges = [{'start': a * TICKS_PER_SAMPLE, 'duration': (min(b, len(audio)) - a) * TICKS_PER_SAMPLE} for a, b in intervals if min(b, len(audio)) > a]
    details = {'method': 'silero-vad-two-pass-and-rms', 'noise_floor_dbfs': floor, 'rms_threshold_dbfs': threshold,
               'speech_probability_cutoff': SPEECH_CUTOFF, 'speech_guard_ms': 40, 'speech_guard_before_ms': 40, 'speech_guard_after_ms': 220, 'frame_ms': 10,
               'speech_regions_seconds': [(a / SAMPLE_RATE, min(b, len(audio)) / SAMPLE_RATE) for a, b in masks_to_intervals(protected)],
               'vad_normal': normal.tolist(), 'vad_boosted': boosted.tolist(), 'frame_dbfs': db.tolist()}
    return ranges, details
