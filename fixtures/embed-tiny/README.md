# A synthetic Model2Vec model, 4 KB

Generated, not trained. Fifty words drawn from the reference corpus's own language, and
sixteen-dimensional vectors derived from a fixed hash of each word's letters.

It exists so CI can exercise the `Semantic` code path — loading, embedding a store, ranking a
query, staying deterministic — without downloading 30 MB. Before it, that whole path ran only
by hand, which is the same shape as a fuzz target nobody runs, and this project has been
bitten by that twice.

**It is not a quality benchmark and must never be used as one.** The vectors mean nothing:
two words are near each other because their letters are, not because they are related. What
retrieval quality actually is gets measured by `make eval-semantic` against a real model, and
those numbers live in the changelog.

Regenerate with the script in the 0.8.0 commit that added this directory. Nothing depends on
the exact values, only on their being stable.
