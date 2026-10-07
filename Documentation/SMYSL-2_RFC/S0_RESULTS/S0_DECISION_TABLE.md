# S0 decision table (RFC SMYSL-2.2 Section 3.7)

Rows resting on same-as or holder labels are **provisional**: the labels are a
model's, not two people's, so the human ceiling GE-T9 asks for is still unmeasured.

| # | measure | threshold | measured | state | plan change |
|---|---|---|---|---|---|
| **T1** | alpha (same/not), en and ru | < 0.667 after one revision | 0.600, 95% CI [0.550, 0.646], 3 model coders (deepseek, gemini, ollama) | FIRES | gold unreliable; restate GE-T5/GE-T2 against alpha |
| **T2** | J_class within model, median over judged inputs | >= 0.6 for both models | L 1.000, R 0.327 | does not hold | - |
| **T3** | same | < 0.6 for either model | L 1.000, R 0.327 | FIRES | pivot to consensus extraction (attested:2) |
| **T4** | same | < 0.4 for both models | L 1.000, R 0.327 | clear | - |
| **T5** | J_class narrative (P3) vs law/epistle (P1, P7) | narrative < 0.6 while others >= 0.6, or RA on narrative under H > 25% | narrative 0.747, law/epistle 0.811, RA(H,narr) not measured | clear on J_class; the RA disjunct is unmeasured (arm H produced no probe unit in Gen.4) | - |
| **T6** | precision at recall 0.7, within language | < 0.9 | C2 cosine 0.789, C3 and C2 cosine 0.835 | FIRES | class measures ship as exploration only; TX-P7 defaults to attested:2 |
| **T7** | precision at recall 0.7, cross-lingual | < 0.9 | C2 cosine 0.427, C3 and C2 cosine 0.712 | FIRES | cross-lingual classes wait for TX-P10 S1 embeddings; anchored only |
| **T8** | RA rate, arm L | > 10% | deepseek 24.5% of 147, gemini 11.1% of 126 | FIRES | holder/mode structural from TX-P5; probe gate in its exit tests |
| **T9** | RA rate, arm H | <= 5% where L > 10% | deepseek 23.8% of 21, gemini 16.7% of 24 | does not hold (arm H is above 5% on every coder) | - |
| **T10** | V1 false-Absent, ru (M3b) | > 1% of quotes | 0 of 1575 = 0.00% | clear | - |
| **T11** | E022 degradations, ru | > 5% of units | 161 degraded units (W304) against 1785 = 9.0%, from 552 E022 events | FIRES | confirms FC-6 as a precondition; ru A1 not comparable until F-2 lands |
| **T12** | GE-T3 pilot | between-language variance > run variance | L between 51.9 / within 239.7; R between 60.5 / within 249.3 | clear | - |
| **T13** | ru/en model-token ratio vs estimator ratio | differ by > 20% | L regression 3.53 vs 2.03 (74%); L totals 2.07 vs 1.96 (6%); R regression 1.99 vs 2.03 (2%); R totals 3.94 vs 1.96 (101%) | FIRES (R); the regression disagrees on L, R | calibrate FC-6 weights on M5 data |
| **T14** | measured tokens per chapter | > 2x the planning figure | H-de 1.24x, H-en 1.28x, H-es 1.22x, H-fr 1.92x, H-ru 1.61x, L-de 0.71x, L-en 1.11x, L-es 1.00x, L-fr 1.59x, L-ru 1.69x, P-en 1.62x, P-ru 3.19x, R-de 2.17x, R-en 1.42x, R-es 1.27x, R-fr 1.34x, R-ru 3.96x, T-en 1.24x | FIRES (P-ru, R-de, R-ru) | revise the cost model before TX-P5 is sized |

- fires: T1, T3, T6, T7, T8, T11, T13, T14
- inconclusive: none
- not measured: none
