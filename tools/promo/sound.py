"""Synthesizes the soundtrack of the promo video (no samples, only numpy).

Usage: python3 tools/promo/sound.py out.wav
The timeline matches tools/promo/film.py: title 0-2.6 s, race from 3.0 s for as long as JasperReports
takes (J_END in site/index.html, 2.63 s), verdict, end card at 9.0 s. Everything sits on a 120 bpm
grid, so the race starts on a beat.
"""
import sys
import wave

import numpy as np

SR = 44100
LEN = 12.2
N = int(SR * LEN)
L = np.zeros(N)
R = np.zeros(N)
rng = np.random.default_rng(7)


def add(sig, at, pan=0.0, gain=1.0):
    i = int(at * SR)
    if i >= N:
        return
    sig = sig[: N - i] * gain
    L[i:i + len(sig)] += sig * np.sqrt((1 - pan) / 2)
    R[i:i + len(sig)] += sig * np.sqrt((1 + pan) / 2)


def t(dur):
    return np.arange(int(dur * SR)) / SR


def lowpass(x, cutoff):
    """One-pole low-pass; cutoff may be an array (a sweep)."""
    a = 1 - np.exp(-2 * np.pi * np.broadcast_to(cutoff, x.shape) / SR)
    y = np.empty_like(x)
    acc = 0.0
    for i in range(len(x)):
        acc += a[i] * (x[i] - acc)
        y[i] = acc
    return y


def tone(freq, dur, attack=0.005, decay=None, harmonics=(1.0,), detune=0.0):
    x = t(dur)
    out = np.zeros_like(x)
    for k, amp in enumerate(harmonics, start=1):
        out += amp * np.sin(2 * np.pi * freq * k * x)
        if detune:
            out += amp * np.sin(2 * np.pi * freq * k * (1 + detune) * x + 1.0)
    env = np.minimum(1, x / attack)
    env *= np.exp(-x / decay) if decay else np.minimum(1, (dur - x) / 0.4)
    return out * env


def kick(gain=1.0):
    x = t(0.32)
    f = 46 + 90 * np.exp(-x / 0.03)
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-x / 0.11) * gain


def flick(pitch):
    """A sheet of paper landing: a short burst of filtered noise with a pitched click."""
    x = t(0.07)
    n = rng.standard_normal(len(x))
    n = n - lowpass(n, 1800 + 2500 * pitch)
    return n * np.exp(-x / 0.012) * 0.5 + np.sin(2 * np.pi * (700 + 900 * pitch) * x) * np.exp(-x / 0.02) * 0.5


def whoosh(dur, lo, hi, rise=True):
    x = t(dur)
    sweep = lo * (hi / lo) ** (x / dur if rise else 1 - x / dur)
    n = lowpass(rng.standard_normal(len(x)), sweep)
    env = (x / dur) ** 2 if rise else np.sin(np.pi * x / dur) ** 2
    return n * env


D2, A2, D3, A3, D4, E4, FS4, A4, D5, E5, FS5, A5 = 73.42, 110.0, 146.83, 220.0, 293.66, 329.63, 369.99, 440.0, 587.33, 659.25, 739.99, 880.0
WARM = (1.0, 0.45, 0.2, 0.08)

# --- pad: one warm chord under everything, swelling into the race ---
for f, pan in ((D3, -0.3), (A3, 0.3), (E4, -0.1), (D2, 0.0)):
    x = t(LEN)
    pad = tone(f, LEN, attack=1.2, harmonics=WARM, detune=0.004)
    swell = 0.35 + 0.65 * np.clip((x - 2.6) / 3.4, 0, 1)
    add(lowpass(pad * swell, 900), 0, pan, 0.045)

# --- title: a soft whoosh in, then an accent when "from any language." lands ---
add(whoosh(0.7, 300, 5000, rise=False), 0.0, 0.0, 0.10)
add(kick(0.7), 0.05)
for f, pan in ((D4, -0.4), (A4, 0.4), (FS5, 0.0)):
    add(tone(f, 1.6, decay=0.5, harmonics=WARM), 0.72, pan, 0.10)
add(kick(0.9), 0.72)

# --- race, from 3.00 s: beat, Jasper's slow ticks, ferroreport's flurry, a riser ---
# The race's timings, as in site/index.html: J_END, F_END and the start of each of Jasper's steps.
RACE = 3.0
J_END, F_END = 2.63, 0.0112
JASPER_STEPS = (0.0, 0.23, 0.40, 1.91, 2.33)
for beat in range(int(J_END / 0.5) + 1):
    add(kick(0.9), RACE + beat * 0.5)
    add(tone(D2, 0.45, decay=0.2, harmonics=(1, 0.5, 0.2)), RACE + beat * 0.5, 0, 0.20)
for i in range(int(J_END / 0.25)):  # offbeat hats, getting brighter
    x = t(0.05)
    hat = rng.standard_normal(len(x))
    hat = (hat - lowpass(hat, 6000)) * np.exp(-x / 0.012)
    add(hat, RACE + 0.25 + i * 0.25, 0.25, 0.04 + 0.006 * i)
for at in JASPER_STEPS:  # Jasper's log lines: dull, left
    add(tone(196, 0.18, decay=0.05, harmonics=(1, 0.3)), RACE + at, -0.7, 0.16)
for k in range(3):  # ferroreport's three steps: done before you hear the first one end
    add(tone(1320 * (1.12 ** k), 0.12, decay=0.03), RACE + 0.01 + k * 0.035, 0.7, 0.14)
steps = int((J_END - F_END) / 0.075)
for k in range(steps):  # one flick per sheet on screen, climbing
    add(flick(k / steps), RACE + 0.01 + k * 0.075, 0.55 + 0.25 * np.sin(k), 0.12 + 0.08 * k / steps)
add(whoosh(J_END, 200, 9000), RACE, 0.0, 0.09)
arp = (D4, FS4, A4, D5, E5, FS5, A5)
for k in range(int(J_END / 0.125)):  # an arpeggio that climbs with the counter
    add(tone(arp[k % len(arp)] * (2 if k >= 14 else 1), 0.2, decay=0.07, harmonics=(1, 0.3)), RACE + k * 0.125, 0.3, 0.035 + 0.003 * k)

# --- Jasper's single invoice lands; then the verdict ---
add(tone(98, 0.5, decay=0.12, harmonics=(1, 0.6, 0.3)), RACE + J_END, -0.6, 0.35)
HIT = RACE + J_END + 0.39
add(kick(1.3), HIT)
x = t(1.2)
add(lowpass(rng.standard_normal(len(x)), 500) * np.exp(-x / 0.35), HIT, 0, 0.25)  # low boom
for f, pan in ((D2, 0), (D3, -0.2), (A3, 0.2), (FS4, -0.4), (A4, 0.4), (E5, 0.0), (FS5, 0.3)):
    add(tone(f, 3.2, decay=1.1, harmonics=WARM, detune=0.003), HIT, pan, 0.085)

# --- 9.0 s: end card ---
add(whoosh(0.45, 400, 8000), 8.6, 0.0, 0.12)
add(kick(1.2), 9.05)
for f, pan in ((D2, 0), (A2, 0), (D3, -0.3), (A3, 0.3), (FS4, -0.2), (D5, 0.2), (A5, 0.0)):
    add(tone(f, 3.1, decay=1.3, harmonics=WARM, detune=0.003), 9.05, pan, 0.085)
for k, f in enumerate((A5, FS5 * 2, D5 * 4)):  # three bright notes: the logo
    add(tone(f, 1.2, decay=0.35), 9.35 + k * 0.11, 0.2 - 0.2 * k, 0.05)

# --- master: gentle saturation, fades, normalize ---
mix = np.stack([L, R], axis=1)
mix = np.tanh(mix * 1.6)
x = np.arange(N) / SR
mix *= (np.minimum(1, x / 0.05) * np.minimum(1, (LEN - x) / 1.2))[:, None]
mix *= 0.89 / np.abs(mix).max()
with wave.open(sys.argv[1], "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes((mix * 32767).astype("<i2").tobytes())
