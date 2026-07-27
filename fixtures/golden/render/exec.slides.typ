#set page(width: 25cm, height: 14cm, margin: 1.5cm)
#set text(size: 20pt)
#set document(title: "Pool saturation is the leading cause; the clean canary is unexplained")

#align(center + horizon)[
  #text(size: 28pt)[Pool saturation is the leading cause; the clean canary is unexplained]

  #text(size: 14pt)[brief · exec]
]
#pagebreak()

// slide 1: b3:qq2fbg6tjizz625cqorddxhhft
#text(size: 14pt)[bottom-line]

#strong[⊢] Pool saturation is the leading cause

Connection wait time tracks the latency curve within the noise floor.

#block(stroke: 1pt, inset: 6pt)[contested — k/pool-vs-canary: contested, 1 position(s) on record]

#pagebreak()

// slide 2: b3:ukzwrh3ou5hyyf7ckp5yrjya5g
#text(size: 14pt)[support]

#strong[▪] Consequently, p95 request latency rose from 180ms to 410ms after the 4.2 rollout

// speaker note: metric: p95\_request\_seconds
#pagebreak()

// slide 3: b3:chktjhx3padm6ehypfaduwukik
#text(size: 14pt)[risk]

#strong[?] The canary shard was clean throughout

#block(stroke: 1pt, inset: 6pt)[contested — k/pool-vs-canary: contested, 1 position(s) on record]

#pagebreak()

// slide 4: b3:py2mbw2ojee3ehhhosiu6vairs
#text(size: 14pt)[ask]

#strong[≈] Roll the eu-west shard back to 4.1 and re-measure


#pagebreak()
#strong[Open contentions:] k/pool-vs-canary
