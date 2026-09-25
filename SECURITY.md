# Security

## Reporting a problem

Email **daniel@themalwarefiles.com**. Put "stegobench" in the subject line so
it doesn't get lost.

Please don't open a public issue for anything that looks exploitable. A
private email first gives everybody time to fix it before it's described in
public.

Useful things to include, if you have them: what you did, what happened, the
commit you were on, and the output of `stegobench doctor --json`.

**What to expect.** This is a one person project maintained alongside other
work, so there's no security team and no rota. Your report is read by a human
who replies when he next sits down to it. There's no guaranteed response time
and this file isn't going to invent one; if your disclosure is on a clock, say
so in the first message and you'll get an honest answer about whether that
deadline can be met.

There's no bug bounty. Credit in the release notes if you'd like it, or no
credit at all if you'd prefer that.

## What's supported

Nothing has been tagged yet, so there's one supported version: the current
`dev` branch. Fixes land there.

## The actual threat surface

This is a tool that runs other people's software over files you give it, so
it's worth being plain about what that means rather than writing the usual
boilerplate.

**A registry entry is an instruction to run code.** Every detector and
embedder under `plugins/registry/` names either a container image or a program
already installed on your machine, plus the argv used to invoke it. Adding a
registry entry from an untrusted source is the same decision as running a
script from that source. Read an entry before you use it, and treat a pull
request that adds one as a code change rather than a configuration change.

**The images are third-party steganalysis tools.** They parse image files that
you supply, and some of them are old C programs doing exactly the kind of
parsing that memory-safety bugs live in. If you're pointing this at images
from a case, from the internet, or from anywhere you don't control, assume
those parsers are hostile input handlers and run the whole thing somewhere you
can afford to lose.

**Containers are the isolation, and they're not perfect.** When the harness
runs a containerised tool, whether that's `stegobench doctor` putting one
through its self test or `stegobench score` walking it over a whole corpus, it
does so with the network switched off, every capability dropped, no new
privileges, a read-only root filesystem, a 2 GB memory cap, the image under
test mounted read-only, and a deadline that kills a run that hangs. That's a
meaningful reduction in blast radius; it isn't a
guarantee against a container escape, and a kernel bug defeats all of it. For
genuinely untrusted material, put a virtual machine or a separate host between
the work and anything you care about.

**Binary entries have no isolation at all.** A `[binary]` registry entry runs
a program straight on your machine as you, because the entry is a statement
that you installed that program deliberately. That's a documented limit rather
than an oversight, and it's the reason a binary entry deserves more scrutiny
than a container one.

**Secrets.** A registry entry records the *names* of environment variables a
tool needs and never their values. A credential written into an entry's `env`
or `secrets` field is refused outright, because that file gets committed.
Nothing in this repository should ever be a place a key lives.

**Results name what produced them.** A `result-v1` document carries each
plugin's image digest (a mutable tag is refused there too), the seed where one
was recorded, and whether the run could reach the network. If you
are consuming results from somebody else, those fields are what you check
before you believe the number.

## Verifying a release download

Release artefacts are signed with keyless Sigstore cosign and carry a GitHub
build provenance attestation. The two commands are `gh attestation verify` for
the provenance and `cosign verify-blob` for the signature; the README has both
of them in full, under "Verifying a download". This section is what they mean.

**What a signature here proves.** That the file came out of the release
workflow in this repository, on a tag, and that it hasn't changed since. The
certificate names the workflow file and the ref, and it's recorded in
Sigstore's public transparency log, so a forged signature would have to be
published somewhere anybody can read.

**What it doesn't prove.** That the code is good, that the release does what
the notes say, or that the account that pushed the tag wasn't compromised. A
signature is a statement about origin, not about quality or intent. If somebody
takes over the repository, what they publish will verify: what you'd notice is
the identity in the certificate, which is why the verification command names it
explicitly rather than accepting any signature at all.

**There is no private key.** Both mechanisms use the workflow run's own
short-lived identity, so there's nothing stored in a secret for an attacker to
take and nothing for the maintainer to lose. That also means there's no key to
fetch before verifying and none to rotate afterwards.

**`SHA256SUMS` is signed too.** It's the file an attacker would most want to
replace, because a checksum list proves only that the bytes match a list the
same attacker could have written. Verify it with `cosign verify-blob` first,
then trust `sha256sum -c`.

**If verification fails**, don't run the binary. Say so by email (above) with
the file you downloaded, where you got it and the exact command output.

## What's out of scope

- The published corpus content itself. Pentimento is built from permissively
  licensed covers, and questions about a specific image's provenance belong in
  its own repository rather than here.
- Findings that require an attacker to already have the ability to write to
  your registry directory or to your `PATH`. At that point they can run code
  on your machine without going through this tool.
- Denial of service against a tool you chose to run over an input you chose to
  give it. A detector that takes an hour on a hostile image is a detector
  problem, and the harness's answer to it is the deadline.

## Dependencies

The Rust dependency set is deliberately small, every dependency is reviewed
before it's added, and lockfiles are committed. CI runs `cargo audit` and
`pip-audit` on every push and every pull request, and fails the build rather
than printing a warning.

An audit tool catches what's already published as an advisory, which is why
the seven day cooldown in `renovate.json` sits alongside it: most compromised
releases are caught and pulled within days of going up.
