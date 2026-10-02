# Changelog

Everything notable that has landed in Stegobench, written for somebody who
wants to know what the tool does today. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and dates are ISO
8601 (`YYYY-MM-DD`).

## [1.0.0] — Pilot — 2026-10-03

The first release. Everything below is what the tool does today; nothing in
it is a promise about what comes next.

### CLI

- Every command in the tree is built and tested: `list`, `describe`,
  `doctor`, `examine`, `schema`, `validate`, `verify`, `plan`, `score`,
  `metrics`, `fetch`, `report`, `completions` and `help`.
- `stegobench examine` runs the detectors you name over images of your own and
  prints one row per image and one column per detector, so several tools can be
  compared on the same files without installing or invoking any of them
  yourself. Name a directory and it stands for the images inside it.
- An examination writes no result document, and nothing it prints is quotable
  as an accuracy. Your own images carry no labels, so there is nothing for a
  detector to be right or wrong about; `stegobench help scope` has the
  difference and `score` is what produces a number you can defend.
- `check`, `scan`, `inspect`, `detect` and `analyse` run `examine`. They used
  to be refusals that explained the tool could not look at your own images.
- Asking for a payload back out of an image is still refused, and the refusal
  now says that rather than talking about scope: `decode`, `extract`, `reveal`
  and `unhide` explain that Stegobench asks detectors questions and does not
  pull hidden data out of pictures.
- zsteg runs under the sandbox again, and its answers are its own rather than
  its guesses. It's asked with `--no-file --no-strings`, so it reports what it
  finds in the container and no longer runs the `file` command over every
  extracted bitplane: that was what wrote a temporary file the sandbox forbids,
  and what reported random bits from a clean photograph as an encryption key.
- A detector that crashes is reported as having failed, not as having found
  the image clean. A tool that exits without answering used to produce the
  same cell as a tool that looked and found nothing, so four crashed runs
  could read as four clean images with the command still exiting 0.
- A detector asked about a format its own registry entry doesn't declare
  still answers, and the table now says underneath which images those were
  and that its answers about them are outside what it claims to support.
- `stegobench examine --json` records the SHA-256 of every image it examined,
  with the tool name and version beside them, so an examination can be filed
  as a record of which bytes were looked at rather than which filenames.
- `stegobench doctor` says `undrivable` rather than `present` for an entry
  that declares no command to launch.
- `stegobench fetch --out` names where a corpus goes. It was `--dest` while
  `embed`, `score` and `report` all said `--out`, so the README and the corpus
  guide both documented the name the rest of the tree teaches and the first
  command a reader copied was refused. `--dest` keeps working.
- `stegobench describe` and `stegobench list` now say when a registered entry
  is one nothing here can drive. An entry can be registered for its licence,
  its provenance and its cost and still declare no command to launch, and two
  commands used to call such an entry runnable while two others refused it.
- An examination says when a detector gave one answer to every image. That is
  what finding nothing looks like and also what a misread output looks like,
  and the table cannot tell you which; `--raw` can.
- An examination's rows carry their parent directory when two images share a
  filename, so a cover and a stego image both called `000000.png` are two rows
  a reader can tell apart.
- `stegobench report` turns a folder of result documents into a table for an
  evaluation document, as text, Markdown or CSV. Every row carries the corpus
  and its digest, the configuration, the pairing and the split beside the
  number, so a figure can't be copied out of the table without them.
- `stegobench report` refuses to build a misleading table: two corpora never
  share one, a `custom` run never sits among `named` ones, nothing is ordered
  by score, and a file that isn't a valid result document is named at the top
  of the report with its reason rather than quietly missing from it.
- `stegobench verify` recomputes the corpus digest a result claims and says
  whether the document and the corpus are about each other. A number somebody
  sends you can now be checked rather than believed.
- A result names the corpus it was measured on by digest, computed from what
  the corpus's own records declare, so it survives the corpus being extracted
  from a shard and moved.
- The arm a result describes is read off the corpus instead of assumed. It
  used to say spatial and PNG on every run, which was wrong for every JPEG
  arm and looked exactly like a fact.
- `stegobench score` runs a registered detector over a corpus directory and
  writes a validated `result-v1` document. The run resumes after an
  interruption, and refuses to continue if the corpus changed under the
  records rather than filing answers against the wrong images.
- `stegobench score --jobs N` scores N images at once. It defaults to 1
  because each worker starts its own container or process, and several
  competing for one machine can make a tool fail in ways that look like a
  detection result.
- `stegobench score --keep-raw` keeps what the detector printed for every
  image, in `<records>.raw.jsonl`, rather than only for the ones no answer
  could be read from. It is what an adapter is debugged on.
- `stegobench score` checks the two reliability rules rather than declaring
  them. A corpus that puts a cover and its stego twin on opposite sides of a
  train and test split stops the run, because that inflates every number and
  the inflation is invisible afterwards.
- Pairing is checked as far as two files on disk allow: for every stego image
  that names its cover, the two headers are compared for format, size, bit
  depth and channel count, and a difference marks the run confounded and names
  the images. A run that could compare nothing says so rather than reading as
  the good case.
- `stegobench list` names the route each tool takes, container or local, and
  says once underneath what each costs you. A result records two separate
  fields beside the digest, `pinned_by` and `isolation`: one says what ties the
  numbers to particular bytes, the other says whether the tool was sandboxed,
  ran on the host, or answered over the network. A single `route` word could
  not say both, and a document still carrying it is refused rather than
  half-read.
- `stegobench metrics` turns a file of scores and labels into ROC AUC and
  detection rates at a false-alarm budget you choose. It's the one
  implementation of that arithmetic, so anything that can start a process and
  write JSON gets the same numbers `score` does. An image the detector could
  not score is refused by name rather than counted as a zero.
- `stegobench fetch` downloads one tier of a registered corpus and checks what
  arrives against the URL, the SHA-256 and the exact size the registry declared
  in advance. A corpus whose terms don't permit redistribution is refused before
  a connection opens. No registered corpus declares a download route yet, so the
  command has nothing to fetch today and says so.
- A registry entry can state the platforms a locally installed tool exists
  for. `stegobench doctor` then says it cannot run here, separately from its
  count of tools you have not installed.
- A result records the operating system and the architecture it ran on.
- `stegobench score --corpus-id` names the registered corpus a directory
  holds, and the harness checks the claim instead of taking it. A named run
  also hashes every image against the digest its own record states, so a real
  manifest with easier images underneath it is refused. A run is
  `named` only when the corpus entry declares the digest of its records and
  the directory matches it; a directory that doesn't is refused before
  anything is scored.
- `stegobench score` reads an unpacked corpus directory. A run over one whose
  registry entry declares no digest is marked `custom`, which is comparable
  with itself rather than with anybody else's number.
- Each way a run can fail now exits with its own documented code instead of
  all of them reporting a plugin failure.
- `stegobench plan` estimates what a run would cost. It takes the command you
  would type rather than its own flags, so a plan cannot describe a different
  run from the one that would happen.
- `stegobench doctor` runs each registered tool's self test in both
  directions, so a detector that answers "stego" to everything fails the
  check instead of passing it.
- Every subcommand accepts `--json`, which puts machine-readable output on
  stdout and leaves progress and diagnostics on stderr.
- Exit codes are fixed and documented on `--help`: 0 success, 1 generic
  failure, 2 usage error, 3 pre-flight refusal, 4 plugin failure, 5
  verification mismatch, 6 schema invalid, 7 licence refusal, 8 environment
  unfit, 130 interrupted.
- `stegobench help <topic>` carries the conceptual reasoning that doesn't fit
  on a flag: `scope`, `pairing`, `splits`, `licences`, `plugins`, `results`
  and `reports`.
- `stegobench help results`, and a guide page beside it, say which fields of a
  result to check before believing the number in it, in the order of what it
  costs to be wrong.
- Man pages are generated from the same command tree the binary parses
  against, so the two can't drift apart.

### Engine

- Three published schemas, `result-v1`, `run-v1` and `manifest-v1`, generated
  from the Rust types that write them rather than maintained by hand.
- A registry reader that refuses a container image named by a mutable tag,
  because a tag can be repointed after a measurement was taken.
- Metrics (ROC AUC, detection at a fixed false-alarm rate, confusion counts)
  live in a crate with no dependencies, so a number can be checked without
  auditing a dependency graph first.
- A plugin host that probes availability, parses tool output and runs the
  registered self tests under a deadline.

### Registry

- A detector entry with an `[invoke]` block now has to declare `[emits]`, and
  is refused at load with the block to add if it doesn't. Nothing can tell
  from a number whether a high one means stego, so the assumption made in
  place of a declaration was one of the two ways a measurement comes out with
  its sign reversed, and `describe` read it back as though somebody had
  written it down.
- Thirteen tools registered: seven detectors (Aletheia's SPA, RS and
  rich-model estimators, StegExpose, zsteg, plus Stegcore and StegaShield as
  subjects rather than references) and six embedders (steghide, outguess,
  openstego, stegosuite, hstego and Stegcore's embed side).
- Five corpora registered. A licence is recorded as `verified` only when an
  identifier, a link, a date and a note on what was read are all present; four
  of the five clear that bar and BOSSbase is recorded as granting nothing,
  which is a finding rather than a gap.
- Redistribution is recorded as its own field, separate from the licence,
  because a corpus you may use isn't always one you may publish.
- Every tool entry now records that answer too, with the reasoning beside it,
  and neither is accepted without the other. A licence says what we may do
  with the software; whether a built image of it may be republished is a
  separate permission that some of these grants withhold.
- One tool entry records a decision rather than a grant. StegExpose is
  archived and was published by its author for public use, nothing licences a
  copy of it, and the entry says in its own words that the mirror is a
  judgement and what that judgement rests on.
- `stegobench describe` prints both lines, so whoever is about to publish an
  image reads the answer rather than the TOML.
- A registry entry that names a loopback or private-network address in how it
  runs a tool is refused, naming the entry, the field and the address, so a
  benchmark can't ship an address and quietly score whatever answers on it.

### Corpus generators (Python)

- The `pentimento` command line builds a labelled corpus: fetching covers
  with per-file provenance, deduplicating, embedding stego arms, packing
  shards, auditing licences and verifying a release before it's published.
- Cover and stego images are written from the same source array through the
  same code path, so a pair differs in the payload and nothing else.
- Corpus tiers are strictly nested over one deterministic ordering, and the
  train and test split is a property of the cover that every tier inherits.
- Long runs are resumable per arm, with every output file checksummed.

### Docs

- `docs/design/` holds the reasoning that the numbers depend on: the corpus,
  distribution, cover-source licensing and matched pairs.
- The corpus guide says how to get from published shards to a directory
  `score` can read: download, check the digests, extract each shard into its
  own subdirectory under one root, score the root. Members inside a shard are
  named by position, so extracting several into one folder overwrites instead
  of failing.
- `docs/guide/` is the reader-facing path: quickstart, pairing, scores,
  limits, examining your own images and building a corpus.
- `docs/leaderboard.md` publishes the submission rules before any table
  exists, so the rules can be argued about rather than invented once a
  submission arrives.
- A guide page walks a team whose detector answers over HTTP from installing
  the tool to a result document they can quote, which the container and local
  binary routes already had and this one didn't.
- `llms.txt` gives a machine-readable orientation to the repository.

### Release and distribution

- Release artefacts are signed, keyless, with a Sigstore signature beside each
  file and a GitHub build provenance attestation. `SHA256SUMS` is signed too,
  since it's the file an attacker would most want to swap.
- `cargo binstall stegobench-cli` fetches the released binary instead of
  compiling the workspace.
- A Nix flake, so `nix run` gets the same binary a build from source would.
- The Homebrew formula and the Scoop manifest are generated from the release's
  own checksum file rather than hand maintained, so they can't drift from what
  was actually published.
- A post-publish verifier loads every destination the way a stranger would and
  checks that the live page states the release. An upload returning success
  says nothing about what the page shows, and this project has twice found a
  wrong public page days later.
- That verifier also runs nightly, so a takedown or a silent build failure
  surfaces within a day.
- A destination that's serving the release while still marked as one we don't
  check yet now fails that verifier and names itself. Marking a channel
  unchecked is how the list can describe somewhere before it exists; once the
  page answers, leaving it unchecked means a later takedown would be reported
  and pass.

### CI

- The generators are tested on Linux, macOS and Windows, because they're what
  a reader runs. The Rust workspace is tested on Linux and macOS, and compiled
  on Windows. Its tests don't run there yet: a dozen of them drive a shell
  script as a stand in detector, and a suite that skipped its own subject
  would report a pass for having checked nothing.
- A tier smoke test selects Nano, Lite and Core and builds Nano, which proves
  the nesting guarantee without claiming a six-day build ran in CI.
- The documentation site build fails on a dead internal link.
- Every action is pinned by commit hash rather than by tag.

### Security

- Dependency auditing runs in CI for both halves and fails the build rather
  than warning.
- Renovate is configured with a seven day minimum release age, so a version
  compromised and pulled within days never reaches a branch here.

### Other

- Bug fixes and improvements.
