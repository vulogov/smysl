# `cloudevents/1` fixture

`c0.json` is a CloudEvents 1.0 JSON batch, written by hand: three CDEvents deploy events from
one Argo CD source and a merged change from GitHub. Each `type` from one `source` is one stream
(RFC SMYSL-3 §6). `tests/cmd_tel_logs.rs` reads it.
