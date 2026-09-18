<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Steganalysis results as likelihood ratios

Reports detector scores the way a forensic report needs them, next to the way
a steganalysis paper reports them, so the two can be compared on the same data.

Status: **working, validated against a positive control, not yet checked
against NFI's `lir` library.**

## The translation

Steganalysis says: *this detector has an AUC of 0.95.* That describes how a
detector behaves across a corpus and says nothing about the picture in front
of an examiner.

Forensic science says: *this evidence is forty times more probable if the
picture carries a payload than if it does not.* That is a likelihood ratio,
it is about this picture, and it is the form a court can combine with the rest
of a case.

Almost nobody bridges the two. This does.

```
LR = P(score | carries a payload) / P(score | does not)
```

A detector score is not already an LR. Sample Pair Analysis returning 0.43
does not mean the odds are 0.43 or 43 or anything at all, until it is mapped
onto a ratio of two densities estimated from data. That mapping is
calibration, and doing it badly is worse than not doing it: an overstated LR
handed to a court is a miscarriage waiting to happen.

## What is reported, and why three numbers rather than one

**Cllr**, the log-likelihood-ratio cost. Penalises an LR by how wrong it was
and how confidently. Its useful property is a reference point: a system that
answers LR = 1 every time, meaning "this tells you nothing", scores exactly
1.0. AUC has no equivalent.

**Cllr_min**, from the pool adjacent violators fit. What these scores could
achieve if calibrated perfectly. This is discrimination, and it is the
detector's fault.

**Cllr_cal**, the difference. The price of imperfect calibration. This is the
statistician's fault, and it is fixable without touching the detector.

Reporting only Cllr hides which of the two is failing.

## The correction that matters

**The textbook reference of 1.0 is the wrong one, and using it produced three
false findings before it was caught.**

A cross validated pipeline does not score exactly 1.0 on data with no signal.
It errs upward: each fold fits a calibrator to noise, the coefficient comes
out small but not zero, and the LRs scatter around 1 rather than sitting on
it. Cllr is convex with its minimum at LR = 1, so scatter costs. The bias is
about 0.003 to 0.005 on a few hundred cases, and it is systematic, so a
bootstrap over the LRs cannot see it: the bootstrap resamples the output and
the bias is in the procedure.

It was caught by a natural control rather than by reasoning. The `structural`
arm of the round3-q95 corpus appends bytes after the end of a JPEG, which
changes no pixel, so the pixel domain detectors return **byte identical**
scores on that arm and on the clean arm. Verified: `max|diff| = 0.00e+00` for
all three. The true Cllr there is exactly 1.000 with nothing to argue about.
The pipeline reported 1.003 to 1.005 with a bootstrap interval excluding 1.0,
which was about to be published as three detectors performing worse than
silence.

So the reference is measured instead. `cllr_null` permutes the labels, runs
the whole pipeline again, and reports what it produces when there is provably
nothing to find. A result has to beat that, not beat 1.0.

## Results on the round3-q95 JPEG corpus

Every arm, every detector: **no evidential value.** AUCs between 0.476 and
0.512, Cllr indistinguishable from the permutation null.

That is the correct answer. Aletheia's SPA and RS and StegExpose model
bit-level changes to pixels, and steghide and outguess change JPEG
coefficients instead. The detectors were used outside their stated range.

It is also what a broken pipeline would print, which is why the next section
exists.

## The positive control

`positive_control.py` runs the identical pipeline over the spatial LSB corpus
from Stegcore's threshold calibration: 8,000 clean pictures and 36,000 stego.

| arm | detector | n | AUC | Cllr | Cllr_min | Cllr_cal | verdict |
|---|---|---|---|---|---|---|---|
| html/zip | spa | 2334 | 1.000 | 0.051 | 0.023 | 0.028 | informative |
| html/zip | rs | 2334 | 1.000 | 0.032 | 0.010 | 0.021 | informative |
| html/zip | ws | 2334 | 1.000 | 0.030 | 0.010 | 0.020 | informative |
| eth/raw | rs | 9826 | 0.499 | 1.000 | 1.000 | 0.000 | no evidential value |

The pipeline can say both things, so the JPEG table is a measurement rather
than a stuck needle.

**One finding fell out of this that was not being looked for.** On `html/zip`,
where the detector is essentially perfect, `Cllr_cal` is as large as
`Cllr_min`: roughly half the total cost is the logistic calibrator failing to
keep up with a detector that separates the classes completely. For a detector
this good, the calibrator is the bottleneck, and that is invisible to AUC,
which reads 1.000 and stops.

## Pairing

An arm is scored only against the covers it was actually run on. outguess
covers 160 of the 200 pictures; the 40 it never touched are not a control for
it, and pooling them into the clean side moves its AUC five points in the
direction of a more dramatic finding.

This is stricter than the `result-v1` documents for the same corpus, which
pair 200 stego against 197 clean where three covers failed to score. Here
those three arms are dropped from both sides, so `steghide/0500` with
`aletheia_spa` reads 0.512 rather than 0.5147.

## Running it

```
pytest                                  # 24 tests
python analyse_panel.py panel.jsonl     # the LR table for a scored corpus
python positive_control.py              # the control, needs the calibration corpus
```

`numpy` and `scipy` only. NFI's `lir` is deliberately not a dependency: the
next step is to check this implementation against it, and a check against a
library you imported is not a check.

## What is left

- **Validate against `lir`.** Cllr and the PAV decomposition should agree to
  floating point. Blocked on an install.
- **Bounded LRs done properly.** The current bound is a flat clip at 100. The
  empirical lower and upper bound (ELUB) sets it from what the sample can
  actually support, which is the defensible version.
- **A spatial arm in a corpus with a real pairing**, so the LR table and the
  positive control are the same corpus rather than two.
