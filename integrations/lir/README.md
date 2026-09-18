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

## Three corrections, all found by measurement rather than by reasoning

### The floor has to be a floor, and on the same scale

`Cllr_min` was fitted to the raw detector scores and left unbounded, while the
reported ratios were clipped at 100. Two things went wrong at once.

**The clip leaked into the calibration loss.** A floor free to be ten orders of
magnitude more confident than the system it bounds is not a floor. On the
spatial corpus, 68 to 71% of the reported calibration loss was the bound, and
37 to 50% of the total reported Cllr was the arithmetic cost of clipping.

**It was not a floor at all on an inverted detector.** Isotonic regression is
monotone *non decreasing*; a logistic calibrator can fit a negative slope. On a
detector pointing the wrong way, the system beat its own floor and `cllr_cal`
went to **minus 0.50**. That is not hypothetical: outguess inverts StegaShield
to AUC 0.360 on this very corpus. The guarding test iterated separations
`(0.0, 0.5, 1.0, 2.0, 4.0)`, all non-negative, so it could not fail in the
direction the bug lived.

Both are fixed by fitting the floor to the **reported ratios** under the
**same bound**. On the inverted StegaShield arms `cllr_cal` is now +0.040 and
`cllr_min` reads 0.94, correctly recognising that an inverted detector carries
information and is merely pointed backwards.

### The scale of the scores broke the fit silently

The logistic fit ran on raw scores. Offset them by 1e5, which a detector
reporting byte counts or chi-square statistics does naturally, and the
objective saturates, the gradient underflows, and BFGS stops at the starting
point **and reports success**. A genuine one sigma separation with AUC 0.76
came back as Cllr exactly 1.000: a false negative made of arithmetic. Scores
are now standardised before fitting. Measured before and after: coefficient
−7.7e-07 against 1.27.

### The reference point

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

**And it permutes within cover pairs.** A stego picture and the cover it came
from are one unit; the null is that the label within that unit is arbitrary,
not that labels are arbitrary across the corpus. Shuffling freely destroys the
pairing the analysis is built around. On the structural arm:

```
observed                     1.00426
free permutation    mean 1.00214, 5-95% [0.99586, 1.00687], p 0.800
within-pair         mean 1.00425, 5-95% [1.00153, 1.00805], p 0.545
```

The paired null lands on the observed value to five decimals. The free one is
centred 0.002 low and 1.7 times as wide; it errs conservative here by luck.

**One cross validation is a draw, not a measurement.** Across 40 fold seeds on
that arm, Cllr ranged 1.00084 to 1.00870, a spread wider than the bias this
whole apparatus exists to correct. Every Cllr is now the mean over 20 seeds
and is printed with its standard deviation beside it.

## Results on the round3-q95 JPEG corpus

Every arm, every detector: **no evidential value.** AUCs between 0.476 and
0.512, Cllr indistinguishable from the within-pair permutation null at p
between 0.32 and 0.71.

That is the correct answer. Aletheia's SPA and RS and StegExpose model
bit-level changes to pixels, and steghide and outguess change JPEG
coefficients instead. The detectors were used outside their stated range.

It is also what a broken pipeline would print, which is why the next section
exists.

Reproduce with `python analyse_panel.py panel.jsonl`.

## The positive control

`positive_control.py` runs the identical pipeline over the spatial LSB corpus
from Stegcore's threshold calibration: 8,000 clean pictures and 36,000 stego.
Every figure below comes from that script at its defaults.

| arm | detector | n | AUC | Cllr (sd) | Cllr_min | Cllr_cal | verdict |
|---|---|---|---|---|---|---|---|
| html/zip | spa | 2334 | 1.000 | 0.046 (0.001) | 0.036 | 0.011 | informative |
| html/zip | rs | 2334 | 1.000 | 0.033 (0.001) | 0.027 | 0.006 | informative |
| html/zip | ws | 2334 | 1.000 | 0.035 (0.001) | 0.027 | 0.007 | informative |
| ps/raw | rs | 9690 | 0.824 | 0.657 (0.000) | 0.636 | 0.021 | informative |
| eth/zip | rs | 2494 | 0.746 | 0.885 (0.001) | 0.835 | 0.051 | informative |
| eth/b64 | spa | 2494 | 0.557 | 0.997 (0.000) | 0.988 | 0.008 | detectable, not useful |
| eth/raw | rs | 9826 | 0.503 | 1.000 (0.000) | 1.000 | 0.000 | no evidential value |

The pipeline says all three things, so the JPEG table is a measurement rather
than a stuck needle.

**"detectable, not useful" is a verdict this needs.** `eth/b64` with SPA is
distinguishable from the null at p = 0.02 and is worth nothing to an examiner:
Cllr 0.997 against a reference of 1.000. Calling that "informative" because it
cleared a significance test would be this module committing exactly the
overstatement it exists to prevent. A result has to clear the null *and* beat
a stated Cllr to earn the word.

**A claim that used to be here has been withdrawn.** An earlier version
reported that on a near-perfect detector `Cllr_cal` was as large as `Cllr_min`
and concluded that the calibrator was the bottleneck. That was an artefact of
the bound bug described above: the floor was unbounded and the system was
clipped, so the difference between them counted the clip. Corrected,
`Cllr_cal` on those arms is 0.006 to 0.011 against a `Cllr_min` of 0.027 to
0.036, which is a fifth rather than a half, and is not a finding.

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
pytest                                  # 55 tests
python analyse_panel.py panel.jsonl     # the LR table for a scored corpus
python positive_control.py              # the control, needs the calibration corpus
```

`numpy` and `scipy` only. NFI's `lir` is deliberately not a dependency: the
next step is to check this implementation against it, and a check against a
library you imported is not a check.

## What is left

- **Validate against `lir`.** Cllr and the PAV decomposition should agree to
  floating point. Blocked on an install: `lir` pulls in pymc, pytensor, numba,
  llvmlite, optuna, scikit-learn and matplotlib, which is roughly 2 GB.
- **Bounded LRs done properly.** The current bound is a flat clip at 100. The
  empirical lower and upper bound (ELUB) sets it from what the sample can
  actually support, which is the defensible version.
- **A spatial arm in a corpus with a real pairing**, so the LR table and the
  positive control are the same corpus rather than two.
