# Releasing Stegobench

The ordered steps for cutting a tagged release of the Rust half. Most of it is
automated by `.github/workflows/release.yml`; this file is the part no workflow
can do, plus the reading a human has to give the result before anybody can
download it.

Publishing here is close to a one-way door. A crates.io name can't be
unpublished, a published version can't be replaced, and a binary somebody has
already downloaded can't be recalled. Everything below is ordered so the
refusable checks happen before the irreversible ones.

## Before the tag

1. **`dev` is green.** CI passes on the commit you're about to tag, and
   `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`
   and `cargo test --workspace` pass locally.
2. **A panel artefact covers the promoted commits.** Baseline Section 25: the
   six-persona roster in `CLAUDE.md` PART C runs in full, and the synthesis
   lands in `private/reviews/YYYY-MM-DD-panel-<cluster>.md`. The push path
   refuses the promotion without one.
3. **A release gate review is signed off.** Baseline Section 4: integrity,
   general quality, security posture, extreme robustness, over the cumulative
   delta since the last tag. Green CI is necessary and not sufficient.
4. **`DEFERRED.md` is re-read.** Anything promised for "this release" is either
   done or re-scoped in writing.
5. **Bump the workspace version.** `version` in the root `Cargo.toml`, and
   `pyproject.toml` if the Python half moves with it. The version-drift gate on
   `pre-push` checks they agree, and the workflow's pre-flight refuses a tag
   that doesn't match `stegobench-cli`'s version.
6. **Write the CHANGELOG entry.** Baseline Section 18: grouped by area, notable
   first, closing with an `### Other` section whose single bullet is
   "Bug fixes and improvements." The heading carries the version and the
   codename, as `## v1.2.3 — Codename - YYYY-MM-DD`. The workflow extracts this
   section verbatim as the release notes, so what you write here is what the
   release page shows.
7. **Ask the operator for the codename.** Baseline Section 22. It's whatever
   he's watching at the moment, it's never invented, and `pre-push` refuses an
   annotated `v*` tag whose message has no `Codename:` line. The conversation
   is one question: "Picking the codename for vX.Y.Z, what are you watching?"
8. **Confirm the four crate names are free**, the first time only:
   `cargo search stegobench-core` and the other three. A name somebody else
   took is a problem to find now rather than halfway through the publish loop.

## The tag

```sh
git tag -a v1.2.3 -m "Stegobench v1.2.3

Codename: <the one the operator picked>

<one paragraph on what this release is for>"

git push olympus dev v1.2.3
git push origin dev v1.2.3
```

Push to every remote the project has (baseline Section 8). `origin` is the
GitHub repository the workflow runs in, so the tag has to reach it for anything
below to happen.

## What the workflow does, unattended

1. **Pre-flight.** Refuses if the tag and the workspace version disagree, if
   the tag carries no codename, or if the CHANGELOG has no section for the
   version. Nothing expensive builds until all three pass.
2. **Builds four archives:** Linux x86_64 and aarch64 (both musl, statically
   linked), macOS universal (Intel and Apple silicon in one binary), and
   Windows x86_64. Each carries the binary, `README.md`, `LICENSE`,
   `CHANGELOG.md`, and the generated man pages everywhere they mean anything.
3. **Generates an SBOM** per published crate with `cargo-cyclonedx`, CycloneDX
   1.5, JSON.
4. **Writes `SHA256SUMS`** over every artefact, checkable with
   `sha256sum -c SHA256SUMS`.
5. **Attests how they were built.** A GitHub build provenance statement naming
   this repository, this workflow, this commit and this run as what produced
   exactly those bytes.
6. **Signs every artefact, `SHA256SUMS` included**, with keyless Sigstore
   cosign, then verifies each signature against the identity a downloader
   checks. Each file gains a `.sig` and a `.pem`.
7. **Creates a DRAFT GitHub Release** titled `vX.Y.Z — Codename`, with the
   CHANGELOG section as the notes and everything above attached, then compares
   the attached asset list against what was signed on disk.

Steps 5 and 6 run before step 7 on purpose. A failure in either fails the job
with no release in existence, so there's no path that leaves a draft carrying
some signed artefacts and some unsigned ones. The reconcile at the end of step
7 covers the other half of that question: `gh release create` uploads assets
one at a time, and its exit code says less than a comparison of the page
against the directory.

It deliberately does **not** publish to crates.io. That job exists in the
workflow, switched off with `if: false`, because the first publish should be
watched.

## How the artefacts are signed, and why that way

A checksum file answers one question: have these bytes changed since somebody
wrote the list? It cannot answer the question a downloader actually has, which
is whether the bytes came from us, because whoever can serve you a tarball can
serve you a `SHA256SUMS` that matches it. Signing is what separates those two
questions. The decision below was taken while writing the signing path into
`release.yml`, and it is recorded here because the alternative is re-arguing it
at the next release.

Success looks like this: a stranger with no account, no prior contact with the
project and no special tooling can establish that a downloaded archive came
from this repository's release workflow, and the project never holds a key
whose compromise would let somebody else claim the same thing.

| Option | Pros | Cons | Best for |
|---|---|---|---|
| Sigstore cosign, keyless | No private key exists to lose, leak or rotate; the certificate names the workflow, the repository and the tag, so a signature says which run made the file; the Rekor transparency log means a forged signature has to be published somewhere public to work; verification needs one binary and no account | The downloader installs cosign; the verification command is long and gets the identity wrong easily; it depends on Sigstore's public infrastructure staying up; certificates are short lived, so verification relies on the log rather than the certificate's own validity window | Anybody verifying from outside GitHub, including a forensic examiner writing down what they checked |
| Long lived GPG key in an Actions secret | Familiar to anybody who has verified a Linux distribution package; verification works offline once the key is fetched; independent of any hosted service | The private key exists, sits in CI where every workflow change is a chance to exfiltrate it, and cannot be rotated without invalidating the trust anybody built up; key distribution is the unsolved half, since a key served from the same site as the artefacts proves nothing; revocation after a compromise is close to useless in practice | Projects with existing key infrastructure and a distribution channel for the public key that is not the download page |
| GitHub build provenance attestations | First party, so the identity is GitHub's rather than ours; one command, `gh attestation verify`, with no identity regexp to get wrong; records the build, not only the signature, so it says what produced the bytes; no key anywhere | Verification effectively needs the `gh` tool and, for a private repository, an authenticated one; the attestation lives in GitHub's store rather than beside the file, so it does not survive somebody mirroring the tarball elsewhere; it is tied to GitHub as a platform | A downloader who is already on GitHub and wants one short command |

**Recommendation, and what was implemented: both cosign keyless and build
provenance, neither alone.** They fail differently, which is the whole
argument. Cosign's signature travels with the file, so it still verifies from
a mirror, from the Internet Archive, or three years from now off a disk; build
provenance answers "what built this" rather than "who signed this", in one
command, for the majority of downloaders who already have `gh`. Neither puts a
private key anywhere a compromise can reach, and neither depends on the
repository staying in the same hands, which a GPG key does. The GPG option was
rejected on the key: this is a one person project, the key would live in an
Actions secret, and a signing key in CI is a key that is one malicious workflow
edit away from being somebody else's.

`SHA256SUMS` is signed along with everything else, and it is the file that most
needed it.

## Filling in the signing pins

**Two actions in `release.yml` are not pinned yet**, because the session that
wrote this path had no network access and would have had to invent a commit
SHA. A wrong hash that looks right is far worse than an obvious blank, so both
carry the ref `PIN-ME-SEE-RELEASING-MD`, which is not hexadecimal and cannot
be mistaken for a hash:

| Action | What it does | What it needs |
|---|---|---|
| `actions/attest-build-provenance` | Produces the build provenance statement | The full commit SHA of the release being adopted, with the version in a trailing comment |
| `sigstore/cosign-installer` | Puts `cosign` on PATH | The same |

The pre-flight job refuses the release while either is still a placeholder, so
this cannot be forgotten into a tag. To fill them in:

```sh
# The commit a release tag points at, without cloning anything.
# Put the tag you actually intend to adopt in place of <tag>; the version
# numbers are deliberately not written down here, because a version copied
# from a document is a version nobody checked.
gh api repos/actions/attest-build-provenance/git/refs/tags/<tag> --jq '.object.sha'
gh api repos/sigstore/cosign-installer/git/refs/tags/<tag> --jq '.object.sha'
```

Read what changed in that release first, and respect the seven day cooldown in
`renovate.json`. If the tag is annotated the reference
above is a tag object rather than a commit, so dereference it:

```sh
gh api repos/<owner>/<repo>/git/tags/<sha-from-above> --jq '.object.sha'
```

Then write each one as `uses: owner/repo@<40 hex characters>  # vX.Y.Z` and run
the tests below. `tools/release/test_release_workflow.py` refuses a bare tag
anywhere in any workflow, and refuses a commit pin with no version comment
beside it.

While the pins are placeholders, the cosign version itself is also unpinned:
`sigstore/cosign-installer` installs its own default. Pin it at the same time
by passing `cosign-release` to that step.

## Before publishing the draft

Read the draft before making it visible. This is the step that exists because a
published page has twice carried something wrong here for days.

- [ ] Twenty-seven files are attached: nine artefacts (four archives, four
      SBOM files, one per published crate, and `SHA256SUMS`) and a `.sig` and
      a `.pem` for each of them.
- [ ] Download one archive, run `sha256sum -c SHA256SUMS`, unpack it and run
      `./stegobench --version`. The version it prints matches the tag.
- [ ] Verify a signature the way a stranger would, from the downloaded files
      rather than from this checkout, and verify `SHA256SUMS` itself:

      ```sh
      TAG=v1.2.3
      ARCHIVE=stegobench-$TAG-x86_64-unknown-linux-musl.tar.gz

      for f in "$ARCHIVE" SHA256SUMS; do
        cosign verify-blob \
          --certificate "$f.pem" \
          --signature "$f.sig" \
          --certificate-identity-regexp '^https://github\.com/elementmerc/stegobench/\.github/workflows/release\.yml@refs/tags/v' \
          --certificate-oidc-issuer https://token.actions.githubusercontent.com \
          "$f"
      done

      gh attestation verify "$ARCHIVE" --repo elementmerc/stegobench
      ```

- [ ] The identity in that command is the one the README tells a downloader to
      paste. If they have drifted apart, every reader's verification fails and
      the project looks compromised.
- [ ] The notes are the CHANGELOG section and read as intended out of context.
- [ ] The title carries the codename.
- [ ] Nothing in the archive is a path, hostname or file from this machine.

Then publish the draft from the GitHub release page.

## Publishing to crates.io

By hand for now, watched, in dependency order. Cargo refuses a crate whose path
dependencies aren't on the registry yet, so the order isn't optional.

```sh
cargo publish --dry-run --workspace --locked   # packages and verifies, uploads nothing
cargo publish --workspace --locked             # the real thing
```

`cargo publish --workspace` works out the order itself and won't leave two of
the four on the registry if the third is rejected, which is why it's preferred
over four separate invocations.

Trusted Publishing (OIDC from GitHub Actions, no stored token) is the intended
route once the shape of the publish is known to be right. Turning it on is
removing `if: false` from the `crates-io` job, after configuring the four
crates' Trusted Publishing settings on crates.io to name this repository and
the `Release` workflow.

## After publishing

- [ ] `cargo install stegobench-cli` on a machine that has never built this
      project, and run `stegobench doctor`. An install that works only where
      the source tree already is isn't an install.
- [ ] Every URL in the release notes and the shipped `README.md` answers 200.
- [ ] `docs.rs` built all four crates. A failed docs build is silent on the
      release page and loud on the crate page.
- [ ] The crates.io page for each crate shows the right licence, the right
      description, and the README.
- [ ] Record the release in the distribution ledger with channel, URL,
      version, digest and timestamp.

## Known gaps, stated rather than discovered

- **Keyless signing writes to a public log.** That is most of what makes it
  worth having, and it cuts both ways: the repository name, the workflow path,
  the tag and every artefact digest are readable by anybody in Sigstore's
  transparency log from the moment the first signed release is cut, while this
  GitHub repository is still private. Going public is the plan, so this is a
  sequencing note rather than a blocker. Don't cut a signed release before
  you're content for the name to be public.
- **The two signing actions are not pinned yet.** See "Filling in the signing
  pins" above. The release refuses to run until a human has resolved both
  SHAs, so this is a blocked release rather than a silent hole.
- **Nobody has run the signing path end to end.** It is tested offline against
  a stub cosign by `tools/release/test_sign_artefacts.py`, which proves the
  control flow and proves nothing about the real tool's flags. The first
  tagged run is where that gets established, which is another reason the
  release is a draft.
- **The signatures are not listed in `SHA256SUMS`.** The checksum file is
  written before anything is signed, so the `.sig` and `.pem` files are not in
  it. They don't need to be: a signature is checked by verifying it, not by
  comparing it against a list.
- **The macOS archive is not byte-reproducible.** macOS ships bsdtar, which
  has none of GNU tar's determinism flags, so the macOS tarball's metadata can
  differ between two builds of the same commit. The Linux archives are
  reproducible and every archive is covered by `SHA256SUMS`.
- **`LICENSE` isn't inside the published crates.** Each crate declares
  `AGPL-3.0-or-later` and ships the README, but the licence text itself lives
  only at the repository root, which Cargo won't package from outside a crate
  directory. Shipping it needs a copy per crate.
- **The Linux aarch64 build cross-links** with the GNU aarch64 toolchain as the
  link driver while Rust supplies the musl objects. It's the part of the
  workflow most worth watching on a first run.
- **The Python half has no release path here.** `pentimento` publishes to PyPI,
  and the corpus to Internet Archive, HuggingFace and Kaggle through
  `publish-all.sh` and `publish-kaggle.sh`. Those are separate sequences with
  their own verification.
