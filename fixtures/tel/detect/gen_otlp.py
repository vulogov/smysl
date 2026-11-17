"""Write series.otlp.json: series.tsv as one OTLP/JSON export, one gauge per series.

    python3 -I fixtures/tel/detect/gen_otlp.py > fixtures/tel/detect/series.otlp.json

So the detectors can be run end to end, `tel read otlp-json/1` then `tel detect`, over the same
samples GE-M3 pins. Values are copied as their tokens, so nothing passes through a float."""
import json, os
here = os.path.dirname(os.path.abspath(__file__))
points = {}
for line in open(os.path.join(here, "series.tsv")):
    if line.startswith("#"):
        continue
    k, t, v = line.rstrip("\n").split("\t")
    name = k.split("{")[0]
    points.setdefault(name, []).append((t, v))
metrics = []
for name, pts in points.items():
    dps = ",".join(f'{{"timeUnixNano":"{t}000000","asDouble":{v}}}' for t, v in pts)
    metrics.append(f'{{"name":"{name}","unit":"1","gauge":{{"dataPoints":[{dps}]}}}}')
print('{"resourceMetrics":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"ge-m3"}}]},'
      '"scopeMetrics":[{"scope":{"name":"sim","version":"1"},"metrics":[' + ",".join(metrics) + ']}]}]}')
