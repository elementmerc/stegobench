# Scores, not verdicts

A detector that answers "yes" or "no" cannot be plotted.

To compare two detectors you need to know how they trade false alarms against
missed detections, and that trade-off is a curve. A curve needs a number per
image: how suspicious this one looked. A yes or a no is that number after
somebody has already chosen the threshold for you, and thrown the number away.

```
    scores:   0.03  0.11  0.42  0.68  0.91     you can slide a threshold
              └──────── one ROC curve ────────┘ anywhere along this

    verdicts:  no    no    no   yes   yes      one point. No curve.
```

## Why this needs saying

Several detectors, including some reference implementations, compute an
estimate, compare it to a threshold and print a sentence. The estimate exists;
it just never leaves the function.

So where a tool computes a score and discards it, the harness calls the same
function and keeps the number, rather than parsing the sentence and pretending
a boolean is a measurement.

## What gets reported

**Area under the ROC curve.** One number for the whole trade-off. Useful for
ranking, useless on its own for deciding whether to deploy something.

**Detection at a fixed false-alarm rate.** The number that matters
operationally. A detector that finds 90% of stego images while calling one
clean image in ten suspicious is not usable at scale, and an AUC hides that.
Report at 1% and at 0.1%, not bare accuracy.

**Verdict rate**, separately, for the tools that only give a verdict. It is
recorded as what it is: one point on a curve nobody can draw, from a threshold
somebody else chose.

## Cllr, and a warning about it

The log-likelihood-ratio cost, `Cllr`, is the forensic-science measure of how
useful a score is as evidence, and it splits into a discrimination floor and a
calibration penalty.

It is worth knowing that the floor is not free. Fitting the floor on the same
data you measure on is biased upwards, and on pure noise at realistic sample
sizes the "no information" reference sits near 1.0 rather than exactly at it.
A result that clears 1.0 by a few thousandths is measuring that bias. The
integration under `integrations/lir/` carries the decomposition and the
measured reference, and it exists because this bit is easy to get backwards.
