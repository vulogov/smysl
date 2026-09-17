#!/usr/bin/env python3
"""Every internal requirement in `[workspace.dependencies]` names the workspace version, and the
Python and JavaScript implementations carry it.

The crates share one version and are published together, but each is resolved on its own
from crates.io. A requirement left at an older version lets a consumer's lockfile pair a new
crate with an old sibling that lacks what it calls — 1.3's `smysl-ingest` needs
`smysl_core::quote`, and `smysl-core = "1.1.0"` admits 1.2. Every build in this repository
uses paths, so nothing else here can see it.
"""
import json
import re
import sys
import tomllib

with open("Cargo.toml", "rb") as f:
    root = tomllib.load(f)

version = root["workspace"]["package"]["version"]
want = ".".join(version.split(".")[:2])
bad = []
for name, spec in root["workspace"]["dependencies"].items():
    if not isinstance(spec, dict) or "path" not in spec:
        continue
    req = spec.get("version", "")
    got = ".".join(re.sub(r"^[=^~]", "", req).split(".")[:2])
    if got != want:
        bad.append(f"  {name} = \"{req}\", workspace is {version}")

# The three implementations written from the specification version with the workspace too: each
# says which revision of the format document it implements, and that revision is the crate's.
# Nothing checked them, so the JavaScript package said 1.2.0 and the Python one 0.9.0 in 1.4.
with open("python/pyproject.toml", "rb") as f:
    py = tomllib.load(f)["project"]["version"]
if py != version:
    bad.append(f"  python/pyproject.toml version = \"{py}\", workspace is {version}")
with open("nodejs/package.json") as f:
    js = json.load(f)["version"]
if js != version:
    bad.append(f"  nodejs/package.json version = \"{js}\", workspace is {version}")
with open("nodejs/src/index.js") as f:
    m = re.search(r'^export const VERSION = "([^"]+)";', f.read(), re.M)
if not m or m.group(1) != version:
    got = m.group(1) if m else "absent"
    bad.append(f"  nodejs/src/index.js VERSION = \"{got}\", workspace is {version}")

if bad:
    print("dep-versions: behind the workspace version:")
    print("\n".join(bad))
    sys.exit(1)
print(f"dep-versions: every internal requirement names {want}, and python/ and nodejs/ are {version}")
