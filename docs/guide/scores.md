# Scores, not verdicts

A detector that answers yes or no cannot be plotted.

Comparing two detectors means knowing how each trades false alarms against
missed detections, and that trade-off is a curve. A curve needs a number per
image. A yes or a no is that number after somebody else has already chosen the
threshold and discarded the number.

```
    scores:   0.03  0.11  0.42  0.68  0.91     slide a threshold anywhere
              └──────── one ROC curve ────────┘

    verdicts:  no    no    no   yes   yes      one point. No curve.
```

Where a tool computes a score and then prints a sentence, Stegobench calls the
same function and keeps the number rather than parsing the sentence.

## What gets reported

| Metric | What it tells you | What it hides |
|---|---|---|
| AUC | The whole trade-off as one number. Good for ranking | Whether the detector is usable at any threshold |
| Detection at 1% and 0.1% false alarms | Whether it is deployable | Behaviour at thresholds you would never use |
| Verdict rate | All a verdict-only tool can give | Everything, really: it is one point from somebody else's threshold |

Never report bare accuracy. On a corpus that is half clean, a detector that
says "clean" every time scores 50%.

## Cllr, and a warning

The log-likelihood-ratio cost is the forensic-science measure of how useful a
score is as evidence. It splits into a discrimination floor and a calibration
penalty.

The floor is not free. Fitting it on the same data you measure on biases it
upwards, and on pure noise at realistic sample sizes the no-information
reference sits near 1.0 rather than exactly at it. A result that clears 1.0 by
a few thousandths is measuring that bias, not evidence.

`integrations/lir/` carries the decomposition and the measured reference.
