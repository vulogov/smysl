# Structural check fixtures

Each `.smy` file carries exactly one injected structural defect (or none, for a control),
and its `.expected` sibling lists the **exact** code set it must produce — parse
diagnostics and check diagnostics together. The gate is exactness in both directions: an
extra code fails the fixture just as a missing one does, because a checker that
over-reports is as unusable as one that under-reports.

Defects that the constructor already refuses (`SMY-E021`, `E023`, `E031`, `E032`, `E034`)
surface as *parse* diagnostics rather than check findings, because a unit that violates
them never reaches a store. Those fixtures are here too — the code set is what matters,
not which layer produced it.
