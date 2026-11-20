"""Write series.tsv: four series for GE-M3 (RFC SMYSL-3 §26), 240 one-minute samples each.

    python3 -I fixtures/tel/detect/gen_series.py > fixtures/tel/detect/series.tsv

Seeded, so the file is reproducible; the detectors read the file, never this script. Each series
has one thing in it for a detector to find:
  cpu      a level shift from 40 to 55 at minute 150
  latency  a five-minute spike at minute 160
  errors   a dip at minute 170
  queue    a constant 12 that becomes 30 at minute 180
"""
T0 = 1768550400000  # 2026-01-16T08:00:00Z
state = 7
def noise():
    global state
    state = (state * 6364136223846793005 + 1442695040888963407) % 2**64
    return ((state >> 33) % 2001 - 1000) / 1000.0  # [-1, 1], three decimals

def fmt(x):
    s = f"{x:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s

print("# key\tts_ms\tvalue   (written by gen_series.py)")
for m in range(240):
    t = T0 + 60000 * m
    rows = [
        ("cpu", 40 + (15 if m >= 150 else 0) + 1.5 * noise()),
        ("latency", 0.2 + (2.5 if 160 <= m < 165 else 0) + 0.02 * noise()),
        ("errors", 5 - (4 if 170 <= m < 173 else 0) + 0.3 * noise()),
        ("queue", 12 if m < 180 else 30),
    ]
    for k, v in rows:
        print(f'ge_m3_{k}{{instance="sim:9100",job="ge-m3"}}\t{t}\t{fmt(v)}')
