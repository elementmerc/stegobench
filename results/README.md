# What's in here

Measurements this project has actually produced. Two shapes, and the
difference matters.

## `v1/` — twenty-one `result-v1` documents

The published format, one document per arm and detector pair. Each names the
corpus digest it was measured on, what was embedded, and the claims the harness
checked rather than assumed.

Every one of them validates, and there's a test that fails if that stops being
true:

```sh
stegobench validate results/v1/round3-q95-steghide-0500-stegexpose.json
```

To see what's here without opening twenty-one files:

```sh
for f in results/v1/*.json; do
  python3 -c "import json,sys;d=json.load(open(sys.argv[1]));\
print(f\"{d['arm']['embedder']:<12} {d['subject']['name']:<14} AUC {d['metrics']['auc']}\")" "$f"
done | sort
```

Read `stegobench help results`, or
[Reading a result](https://elementmerc.github.io/stegobench/guide/reading-a-result),
before quoting any of these. The short version: they're all marked `custom`,
because these arms aren't a registered tier whose digest anybody declared in
advance, so each number is comparable with itself rather than with somebody
else's.

**The file name is not the schema.** `round3-q95-*` means the third
measurement round, at JPEG quality 95. Every document here is in the same
format whatever it is called.

## Three results were withdrawn

The three `rich-suniward-suniward-0400-*` documents are gone. Their detection
rates were computed by an older piece of code that read the false-alarm budget
differently from the way `stegobench metrics` reads it now, and the scores they
were measured from are not in this repository, so there is no way to work out
here what the corrected figures would be.

A number nobody can recompute is a number nobody can check, which is the thing
this project exists to stop. So they are withdrawn rather than quietly
corrected. They can come back when their scores are on a machine that can
recompute them.

The twenty-one `round3-q95-*` documents were recomputed rather than withdrawn,
because the scores behind them are available. Twenty-six of their detection
rates moved, every one of them upward, by between half a point and three. None
of their AUC figures moved.

## The two loose files — an older shape, kept on purpose

`rich-model-suniward-0400.json` and its pilot are output from
`generators/rich_model_baseline.py`, and they predate `result-v1`. They carry
things that format has no field for: the feature count, the subspace dimension,
the out-of-bag error of the ensemble.

They're kept rather than converted because converting would throw away the
fields that make them worth having, and a reference detector's own training
diagnostics are exactly what somebody reproducing the baseline needs. They are
not `result-v1` documents and `stegobench validate` will say so, correctly.

## What isn't here

No leaderboard, and no ranking. `docs/leaderboard.md` writes down the rules a
table would run under, deliberately before any table exists, so they can be
argued about rather than invented under pressure once a submission arrives.
