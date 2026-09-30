# Scoring a detector that answers over HTTP

Most detectors are commands: hand one a file, read what it prints. Some are
services. They run as a container that serves HTTP, and scoring an image means
posting it to a running instance.

This page is the whole path for that case, for somebody who has never used this
tool before: install it, register your detector, point it at your own instance,
prove it works in both directions, score a corpus, and read the document that
comes out. It assumes you own the detector and want a number you can put in a
report.

## What you need first

| | |
|---|---|
| The detector, running | On a machine this one can reach over HTTP. Yours, on your network; nothing here ever supplies an address |
| The container image it runs from, and its digest | This is what the result names as the subject. `docker pull` prints the digest, and `docker inspect` lists it under RepoDigests |
| Python 3 | Only for the small adapter script that does the posting. It uses the standard library and nothing else |
| The Rust toolchain | To build the harness, unless you have a release archive |

## 1. Install the harness

```sh
git clone https://github.com/elementmerc/stegobench
cd stegobench
cargo build --release -p stegobench-cli
./target/release/stegobench --version
```

Every command below is run from the root of that clone. That matters more than
it usually does: a registry entry that names a script by a relative path has
that path resolved against the directory you are standing in, and a run started
somewhere else fails with `adapter ... not found`. Use an absolute path in the
entry if you would rather not think about it.

Full install notes, including the Python half that builds corpora, are in the
[Quickstart](/guide/quickstart).

## 2. Ask the machine what it can do

```sh
./target/release/stegobench doctor
```

`doctor` walks every registered tool and reports two separate things per line:
whether the tool is **here**, and whether it **works**. Present is not working,
and the output never lets the first stand in for the second. Expect most lines
to say MISSING on a fresh clone; that is a list of things you have not
installed, not a fault.

## 3. Write the registry entry

A detector is a TOML file under `plugins/registry/detectors/`, not code wired
into the harness. A service needs two things an ordinary entry does not:
`host = true`, which says the thing to run is on this machine rather than
inside the image, and an adapter, which is the script that does the posting.

```toml
name = "my-detector"
kind = "detector"
licence = "proprietary"
secrets = ["MY_DETECTOR_LICENCE"]     # names only: never a value

[image]
reference = "example.com/you/my-detector@sha256:..."   # a tag is refused
size_mb = 1250
bundled = false            # derived from the size, not chosen: true at or
                           # below 750 MB, and the entry is refused if the
                           # flag disagrees with its own size

[emits]                    # required once there is an [invoke] block: nothing
                           # can work out from a number which way round it
                           # reads, so an entry the harness can drive has to
                           # say. The entry is refused without it
output = "score"           # a number per image, which is what a curve needs
higher_means_stego = true

[accepts]
formats = ["png", "jpeg"]

[invoke]
host = true                            # run the adapter here, not in the image
adapter = "plugins/adapters/my_detector_one.py"
entrypoint = "python3"
argv = ["{adapter}", "{file}"]
parser = "number"                      # the adapter prints one number
endpoint_env = "MY_DETECTOR_ENDPOINT"  # the NAME of the variable that carries
                                       # your instance's address, never the
                                       # address itself

[selftest]
must_detect = "fixtures/lsb-0.4bpp.png"
must_clear = "fixtures/clean.png"
threshold = 0.5
```

`stegobench describe my-detector` reads it back, and `stegobench list
detectors` shows it beside everything else. To keep your entry outside this
clone, put it in a directory of your own with the same layout and point
`--registry` at it, or set `STEGOBENCH_REGISTRY`. With `plan`, put `--registry`
before the subcommand, because `plan` reads the rest of the line as the command
it is planning. A file the registry refuses stops
every command that loads the registry, with the reason, rather than being
skipped quietly.

### The adapter

`plugins/adapters/stegashield_one.py` is a worked example of exactly this, at
about a hundred lines with no dependencies. Copy it. The contract is small:

- take the image path as the only argument;
- read the endpoint from the environment;
- print one number on stdout and exit 0;
- on any other outcome, print the reason on stderr and exit non-zero.

Never print a number you did not get. An adapter that falls back to zero when a
field is missing reads as the most confident possible "clean", which is the
opposite of not knowing, and it is indistinguishable from a working detector
that found nothing.

### What the image reference is for, and what it is not

The digest is what a result names as the subject. It is how somebody reading
your number can fetch the identical bytes and run them again, which is the
whole point of pinning it, and that is why the entry still requires it.

It is a fact about the artefact rather than about this machine, and the harness
treats it that way. **Your service does not have to run on the machine running
the harness, and the image does not have to be pulled here.** Running it on
another host is the normal arrangement and nothing asks you to duplicate it
locally.

What `doctor` and `score` check instead is the thing they would actually
launch: the adapter file, the interpreter named by `entrypoint`, and whether
you have told them where your instance is. Those three are what a run here
depends on.

The adapter path is resolved against the directory you run from, so a run
started somewhere else reports `adapter ... could not be opened` and names the
path it tried. An absolute path in the entry avoids the question.

## 4. Supply the endpoint

**The address is yours and the entry never carries it.** The harness refuses a
registry entry that names a loopback or private-network address anywhere in its
invocation:

```
my-detector: invoke.env[0] names 172.24.0.2: a private-network address, which
means something different on every network. A benchmark cannot ship an address:
it scores against whatever answers there, and a reader of the result has no way
to know what that was. Leave it out and have whoever runs it supply the address
from the environment
```

Loopback, `localhost`, the link-local range the cloud metadata service sits in,
and the unspecified address are all refused the same way, in argv and in the
container reference as well as in `invoke.env`.

Two reasons, and the second is the one that ruins a measurement. An address in
a file is a default, and a default is scored against whatever answers on it, so
a result can name your detector while measuring whoever happened to be
listening on that port. The other is that your internal addresses are yours,
and a registry entry is a published file.

So export it instead. The harness passes your environment through to the
adapter:

```sh
export MY_DETECTOR_ENDPOINT=http://10.1.2.3:3000/api/score
```

Check the port is yours before you start. 3000 is a common default and
something else on the machine may already hold it, in which case your probes
reach that instead and answer with whatever it serves: a web application
replying `405 Method Not Allowed` to a POST looks nothing like a detector and
costs a while to recognise.

A private address here is entirely fine. It is in your shell, for this run,
rather than in a file that reaches everybody who clones the repository. The
same goes for a licence token: name the variable in `secrets`, and the harness
reports it as missing without ever reading its value.

`endpoint_env` in the entry names that variable so the harness can tell you
about it before a run starts instead of after. The address never goes in the
entry; the variable's name does, and the harness reads the variable only to see
whether there is something in it. An entry that writes an address there is
refused, because `http://10.1.2.3:3000/api/score` is not the name of an
environment variable.

Two situations look similar and are not, so they get different answers:

| What `doctor` says | What it means | What to do |
|---|---|---|
| `unknown   MY_DETECTOR_ENDPOINT is not set to an address` | Nobody has said where your instance is, so there is nothing to ask | Start the service, export the variable |
| `present ... SELF TEST FAILED (could not reach ...)` | The adapter ran and the address did not answer | Check the service is up and reachable from here |

The first stops `score` in pre-flight with exit 3 rather than letting a long
run discover it image by image.

## 5. Prove it works, in both directions

```sh
./target/release/stegobench doctor
```

The fixtures your entry names live in `fixtures/` in the clone, which is where
`doctor` looks unless `--fixtures` says otherwise.

The self-test asks your detector two questions, using the same code path a real
run uses: it must flag `must_detect` and must clear `must_clear`.

**Both are mandatory and a one-sided check is refused**, because a one-sided
check cannot fail. A tool that answers "stego" to everything passes a
detect-only test. A tool that answers "clean" to everything (including one that
crashed and printed nothing usable) passes a clear-only one. This project has
produced four separate controls that could not fail, including an extraction
that ran over 2,000 images, exited zero every time and produced nothing,
because a support package was missing. So the two answers are checked
separately and the failure says which way round it went: *answers yes to
everything*, *answers no to everything*, or *has both answers exactly
backwards*.

`threshold` is per tool, because the outputs are not the same quantity: some
tools return a probability, some an embedding rate, some a fused statistic on
their own scale. It is a smoke-test decision point against a deliberately loud
fixture, and **it is not a calibrated operating point**. Real thresholds come
from a false-alarm budget on a real corpus.

If the endpoint is set but wrong, the self-test fails with the adapter's own
message, naming the address it could not reach. If it is unset, the self-test
is not attempted at all and the line says so instead, because a tool nobody has
pointed anywhere is not a tool that failed.

## 6. Score a corpus

```sh
./target/release/stegobench score \
    --corpus ./pentimento-nano \
    --detector my-detector \
    --out my-detector-nano.json
```

The corpus is an unpacked directory of images with a record beside each: a
published one such as [Pentimento](https://github.com/elementmerc/pentimento),
or one you build yourself with [Build a corpus](/guide/build-a-corpus).

Worth knowing before you start a long one:

| Flag | What it is for |
|---|---|
| `--jobs N` | How many images to score at once. Defaults to 1, one image at a time, because each worker starts its own container or process and several competing for one machine can make a tool fail in ways that look like a detection result. Raise it slowly and watch the machine; above the core count usually buys nothing |
| `--keep-raw` | Keep what the detector printed for every image, not only for the ones no answer could be read from. It goes beside the records in `<records>.raw.jsonl`, and on an 18 image run it was 46 times the size of the records file. This is the flag for writing an adapter |
| `--limit N` | A smoke test over the first N items. The result is marked `custom`, because a prefix of a corpus is not the corpus |
| `--timeout SECONDS` | How long any single image gets before the detector is killed and that item is recorded as an error. Defaults to 60 |
| `--corpus DIR` | A directory of unpacked samples on this machine, never a registered id. `stegobench fetch <id>` is what turns an id into a directory |
| `--corpus-id ID` | Names the registered corpus the directory holds. It is a claim the harness checks, not one it takes |
| `--records FILE` | Where the per-item answers are kept. It may not be inside the corpus: a file written there joins the corpus and the next run measures a different set |

The run is resumable: every answer is written as it is produced, and running
the same command again picks up where it stopped. Interrupt it without losing
the hours already spent.

`stegobench plan score --corpus ./pentimento-nano --detector my-detector`
estimates what a run would cost before you start one, including the worst case
if every item hits the deadline.

## 7. Before you quote the number

The document names the exact bytes it was measured on, and the things that
decide whether the number means anything are fields in it rather than claims in
the prose around it. [Reading a result](/guide/reading-a-result) goes through
them in order; the short version for your own run is:

- `declarations.pairing` should be `single-variable`. Anything else means part
  of what was measured is not the payload. See
  [Pairing](/guide/pairing) for why this is the one that matters most.
- `declarations.configuration` tells you whether the figure is comparable with
  somebody else's. `custom` is honest and is most runs; it rules out quoting
  the figure as a tier number.
- `declarations.trained_on` must not name the corpus you scored on. A detector
  measured on what it trained on is not being measured.
- `provenance.plugins[].isolation` will say `remote-service`, and
  `pinned_by` will say `unpinned`. Your adapter ran here with your network, and
  nothing in the run checked that the instance it talked to was built from the
  image the entry names. The image reference is still recorded; it just isn't
  something this document can vouch for.
- Report AUC and detection at a fixed false-alarm rate. Never report bare
  accuracy: on a corpus that is half clean, answering "clean" every time scores
  50%. See [Scores, not verdicts](/guide/scores).

And read [Limitations](/guide/limits) before the number leaves your hands. It
is the list of things a figure from this harness needs a caveat about, and a
reviewer will find them whether or not you did.

## If you publish the number

[Submitting a result](/leaderboard) has the rules, including the one that
applies to you directly: a number produced by the tool's own authors is marked
self-reported by the submission path, never by the submitter. It is not worth
less for that. It is a different kind of evidence, and saying so is what lets
it be read at all.
