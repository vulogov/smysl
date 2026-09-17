#!/usr/bin/env python3
"""Every internal requirement in `[workspace.dependencies]` names the workspace version.

The crates share one version and are published together, but each is resolved on its own
from crates.io. A requirement left at an older version lets a consumer's lockfile pair a new
crate with an old sibling that lacks what it calls — 1.3's `smysl-ingest` needs
`smysl_core::quote`, and `smysl-core = "1.1.0"` admits 1.2. Every build in this repository
uses paths, so nothing else here can see it.
"""
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

if bad:
    print("dep-versions: internal requirements behind the workspace version:")
    print("\n".join(bad))
    sys.exit(1)
print(f"dep-versions: every internal requirement names {want}")
