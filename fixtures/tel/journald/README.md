# `journald/1` fixture

`j0.json` is `journalctl -o json` output, one entry per line, written by hand in its shape:
eight entries of `cart.service` on host `h1`, 5 s apart from 2026-01-16T10:00:00Z, and one
kernel entry with no `_SYSTEMD_UNIT` (its stream is keyed by `SYSLOG_IDENTIFIER`). The `__`
fields (cursor, monotonic clock) are not part of an event's body, so two reads of one entry
are one event. `tests/cmd_tel_logs.rs` reads it.
