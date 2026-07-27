#set document(title: "Pool saturation is the leading cause; the clean canary is unexplained")
#set text(font: "New Computer Modern", size: 11pt)
#set par(leading: 0.8em, justify: true)

= Pool saturation is the leading cause; the clean canary is unexplained

#emph[brief · profile exec]

#emph[for engineering leadership]

== bottom-line

#strong[⊢] Pool saturation is the leading cause

Connection wait time tracks the latency curve within the noise floor.

#block(stroke: 1pt, inset: 6pt)[contested — k/pool-vs-canary: contested, 1 position(s) on record]

== support

#strong[▪] Consequently, p95 request latency rose from 180ms to 410ms after the 4.2 rollout

#footnote[metric: p95\_request\_seconds]

== risk

#strong[?] The canary shard was clean throughout

#block(stroke: 1pt, inset: 6pt)[contested — k/pool-vs-canary: contested, 1 position(s) on record]

== ask

#strong[≈] Roll the eu-west shard back to 4.1 and re-measure

#strong[Open contentions:] k/pool-vs-canary
