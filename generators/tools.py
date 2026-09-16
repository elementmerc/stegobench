#!/usr/bin/env python3
"""The eight embedders, each with the quirk that makes it different.

Tiers 1 and 2 of the Pentimento tool plan. Tier 3 is the Windows-only family and
tier 4 is learned steganography; both are recorded in Stegcore's roadmap rather
than attempted here.

    tier 1, already had containers or a binary:
        steghide      JPEG and BMP, Blowfish, graph-theoretic LSB, the most cited
        outguess      JPEG, corrects its own statistics after embedding
        openstego     PNG, Java, Random-LSB with a password or Null-LSB without
        stegcore      ours: LSB replacement, adaptive mode, deniable payloads

    tier 2, added 2026-09-16:
        hstego        HILL and J-UNIWARD with REAL syndrome-trellis coding
        stegosuite    Java, AES, LSB across a whole image
        stego_lsb     plain sequential LSB, what a CTF actually uses
        stegano       LSB with several encodings, widely installed

WHY hstego IS THE ONE THAT MATTERS
----------------------------------
Our adaptive arm uses `conseal`, which **simulates** embedding at the optimal
coding rate: it produces the change map a perfect coder would produce. Every
published adaptive result rests on that simulation, and it is slightly harder to
detect than a real coder's output because a real coder cannot hit the bound.

`hstego` implements the same cost functions with an actual syndrome-trellis
coder. Running both over the same covers at the same rates measures the gap
between the assumption and the practice, which the field takes on faith and
nobody publishes. It is the single most interesting column this corpus can add.

WHAT EACH TOOL DOES TO YOUR PAYLOAD WITHOUT SAYING SO
-----------------------------------------------------
This is why `capacity()` exists per tool rather than as one formula:

- **steghide** compresses and encrypts by default, so the bytes written are not
  the bytes handed over, and its capacity depends on the cover's content rather
  than only its size.
- **outguess** rewrites coefficients after embedding to restore the histogram it
  disturbed, which costs capacity it will not tell you about in advance.
- **openstego** writes a header with the original filename in it, so a payload
  named differently changes the file length.
- **stegano** and **stego_lsb** take the bytes literally, which makes them the
  useful baseline the others are measured against.
"""
from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess

from PIL import Image

from embedders import (
    DockerTool,
    EmbedError,
    Embedder,
    EmbedResult,
    run,
    stage,
    temp_workdir,
)


class SteghideEmbedder(DockerTool, Embedder):
    """JPEG and BMP, Blowfish encrypted, graph-theoretic rather than plain LSB.

    Steghide does not overwrite least significant bits in place. It builds a
    graph over pixel pairs whose values could be swapped to carry the wanted
    bits and looks for a matching, so most of the payload is written by
    exchanging samples that already had the right values. That is why the
    histogram barely moves and why it resisted detection for years, until its
    32-bit seed became the attack.
    """

    image = "stegobench/steghide:pinned"
    formats = (".jpg", ".jpeg", ".bmp")

    @property
    def id(self) -> str:
        return "steghide"

    def capacity(self, cover: pathlib.Path) -> int:
        with temp_workdir() as tmp:
            tmp = pathlib.Path(tmp)
            shutil.copy2(cover, tmp / f"cover{cover.suffix}")
            result = self.run_in(tmp, ["info", f"cover{cover.suffix}", "-p", ""])
        for line in (result.stdout or b"").decode("utf-8", "replace").splitlines():
            if "capacity:" in line:
                return _parse_capacity(line.split("capacity:")[1].strip())
        raise EmbedError(f"steghide would not report a capacity for {cover.name}")

    def embed(self, cover, payload, stego, password=None):
        self._check(cover, payload)
        with temp_workdir() as tmp:
            tmp = pathlib.Path(tmp)
            s = stage(tmp, cover, payload)
            result = self.run_in(tmp, [
                "embed", "-cf", s.cover, "-ef", s.payload, "-sf", s.out,
                "-p", password or "", "-q", "-f",
            ])
            produced = tmp / s.out
            if result.returncode != 0 or not produced.is_file():
                raise EmbedError(
                    "steghide refused: "
                    + (result.stderr or b"").decode("utf-8", "replace").strip()
                )
            stego.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(produced, stego)
        return EmbedResult(stego, len(payload), self.id,
                           {"encrypted": True, "compressed": True,
                            "password": bool(password)})


class OutguessEmbedder(DockerTool, Embedder):
    """JPEG only, and it repairs the statistics it disturbs.

    Outguess embeds, then deliberately changes further coefficients to push the
    DCT histogram back towards where it started. That is what defeats a plain
    chi-squared test, and it is also why its usable capacity is roughly half
    what a naive count suggests: the correction has to be paid for out of the
    same coefficients.
    """

    image = "stegobench/outguess:pinned"
    formats = (".jpg", ".jpeg")

    @property
    def id(self) -> str:
        return "outguess"

    def capacity(self, cover: pathlib.Path) -> int:
        # Outguess has no capacity query. The conservative estimate below is
        # deliberately pessimistic: it would rather skip an arm than write a
        # truncated payload, which is silent corruption in a corpus.
        with Image.open(cover) as img:
            width, height = img.size
        coefficients = (width // 8) * (height // 8) * 64
        return max(0, int(coefficients * 0.05) // 8)

    def embed(self, cover, payload, stego, password=None):
        self._check(cover, payload)
        with temp_workdir() as tmp:
            tmp = pathlib.Path(tmp)
            s = stage(tmp, cover, payload)
            args = ["-d", s.payload]
            if password:
                args = ["-k", password] + args
            result = self.run_in(tmp, args + [s.cover, s.out])
            produced = tmp / s.out
            if result.returncode != 0 or not produced.is_file():
                raise EmbedError(
                    "outguess refused: "
                    + (result.stderr or b"").decode("utf-8", "replace").strip()[:300]
                )
            stego.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(produced, stego)
        return EmbedResult(stego, len(payload), self.id,
                           {"statistics_corrected": True,
                            "password": bool(password)})


class OpenStegoEmbedder(DockerTool, Embedder):
    """PNG, Java, and two different algorithms depending on whether you pass a password.

    With no password it writes Null-LSB: sequential, unrandomised, and the
    ground truth for Stegcore's own OpenStego fingerprint. With a password the
    slot order is permuted from a seed, which is the case the v5 brute-force
    work targets. Both are worth an arm and they are not the same tool.
    """

    image = "stegobench/openstego:pinned"
    formats = (".png",)

    @property
    def id(self) -> str:
        return "openstego"

    def capacity(self, cover: pathlib.Path) -> int:
        with Image.open(cover) as img:
            width, height = img.size
            channels = len(img.getbands())
        # One bit per channel per pixel, less OpenStego's own header. The header
        # carries the original filename, so the margin is generous rather than
        # exact.
        return max(0, (width * height * channels) // 8 - 256)

    def embed(self, cover, payload, stego, password=None):
        self._check(cover, payload)
        with temp_workdir() as tmp:
            tmp = pathlib.Path(tmp)
            s = stage(tmp, cover, payload)
            args = ["embed", "-mf", s.payload, "-cf", s.cover, "-sf", s.out]
            if password:
                args += ["-p", password]
            result = self.run_in(tmp, args)
            produced = tmp / s.out
            if result.returncode != 0 or not produced.is_file():
                raise EmbedError(
                    "openstego refused: "
                    + (result.stdout or result.stderr or b"").decode(
                        "utf-8", "replace").strip()[:300]
                )
            stego.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(produced, stego)
        return EmbedResult(stego, len(payload), self.id,
                           {"algorithm": "RandomLSB" if password else "NullLSB",
                            "password": bool(password)})


class StegcoreEmbedder(Embedder):
    """Ours, and in the corpus for the same reason as everything else here.

    A benchmark that judges detectors needs many embedders, and leaving our own
    out would be a different kind of dishonesty from putting it in. The conflict
    that actually mattered was shipping the benchmark from Stegcore's own
    repository, which is why stegobench is a separate project.
    """

    formats = (".png", ".bmp", ".jpg", ".jpeg", ".webp")

    #: Where a release build lands when Stegcore is checked out beside us. The
    #: binary is not installed system-wide on any machine in this fleet, so
    #: without this the arm silently skips and the corpus quietly loses its
    #: most relevant tool.
    SIBLING_BUILDS = (
        "../Stegcore/target/release/stegcore",
        "../../Stegcore/target/release/stegcore",
    )

    def __init__(self, binary: str | pathlib.Path | None = None,
                 mode: str = "adaptive") -> None:
        self.binary = str(binary) if binary else self._discover()
        # `adaptive` is the shipped default and `sequential` is the plain
        # replacement path. They are different embedders to a detector and
        # belong in different arms, so the mode is a constructor argument
        # rather than a hidden default.
        self.mode = mode

    @classmethod
    def _discover(cls) -> str:
        found = shutil.which("stegcore")
        if found:
            return found
        env = os.environ.get("STEGCORE_BIN")
        if env and pathlib.Path(env).is_file():
            return env
        here = pathlib.Path(__file__).resolve().parent
        for candidate in cls.SIBLING_BUILDS:
            path = (here / candidate).resolve()
            if path.is_file():
                return str(path)
        return "stegcore"

    @property
    def id(self) -> str:
        return f"stegcore_{self.mode}" if self.mode != "adaptive" else "stegcore"

    def available(self) -> bool:
        if not (shutil.which(self.binary) or pathlib.Path(self.binary).is_file()):
            return False
        result = subprocess.run([self.binary, "--version"],
                                capture_output=True, check=False)
        return result.returncode == 0

    def capacity(self, cover: pathlib.Path) -> int:
        with Image.open(cover) as img:
            width, height = img.size
            channels = len(img.getbands())
        return max(0, (width * height * channels) // 8 - 1024)

    def embed(self, cover, payload, stego, password=None):
        self._check(cover, payload)
        with temp_workdir() as tmp:
            tmp = pathlib.Path(tmp)
            message = tmp / "payload.bin"
            message.write_bytes(payload)
            # Its own help says a passphrase on the command line is readable by
            # any local user through /proc/<pid>/cmdline while the process runs.
            # A sweep runs this hundreds of thousands of times, so take the file.
            secret = tmp / "passphrase.txt"
            secret.write_text(password or "stegobench")
            stego.parent.mkdir(parents=True, exist_ok=True)
            result = run([
                self.binary, "embed", str(cover), str(message),
                "-o", str(stego),
                "--passphrase-file", str(secret),
                "--mode", self.mode,
            ])
            if result.returncode != 0 or not stego.is_file():
                raise EmbedError(
                    "stegcore refused: "
                    + (result.stderr or result.stdout or b"").decode(
                        "utf-8", "replace").strip()[:300]
                )
        return EmbedResult(stego, len(payload), self.id,
                           {"encrypted": True, "mode": self.mode})


class HStegoEmbedder(DockerTool, Embedder):
    """HILL and J-UNIWARD with a real syndrome-trellis coder, not a simulation.

    This is the arm that measures what the literature assumes. See the module
    docstring: every published adaptive number rests on simulated embedding at
    the optimal coding rate, and this is the same cost function paying the real
    coding overhead.
    """

    image = "stegobench/hstego:pinned"
    formats = (".png", ".jpg", ".jpeg")

    @property
    def id(self) -> str:
        return "hstego"

    def capacity(self, cover: pathlib.Path) -> int:
        with Image.open(cover) as img:
            width, height = img.size
        # hstego reports its own limit on refusal; this is the bound it applies
        # for spatial covers, one bit per pixel less its header and the coding
        # overhead a real STC pays.
        return max(0, (width * height) // 8 - 64)

    def embed(self, cover, payload, stego, password=None):
        self._check(cover, payload)
        with temp_workdir() as tmp:
            tmp = pathlib.Path(tmp)
            s = stage(tmp, cover, payload)
            result = self.run_in(tmp, [
                "embed", s.payload, s.cover, s.out, password or "stegobench",
            ], timeout=300)
            produced = tmp / s.out
            if result.returncode != 0 or not produced.is_file():
                raise EmbedError(
                    "hstego refused: "
                    + (result.stderr or result.stdout or b"").decode(
                        "utf-8", "replace").strip()[:300]
                )
            stego.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(produced, stego)
        return EmbedResult(stego, len(payload), self.id,
                           {"coding": "syndrome-trellis, not simulated",
                            "cost": "HILL for spatial, J-UNIWARD for JPEG"})


class StegoLsbEmbedder(Embedder):
    """Plain sequential LSB, the baseline everything else is measured against.

    No encryption, no permutation, no header games. It is what a CTF challenge
    uses and what a first attempt at steganography looks like, which makes it
    the arm a detector must catch at any payload or it is not a detector.
    """

    formats = (".png", ".bmp")

    def __init__(self, python: str | None = None) -> None:
        self.python = python or "python3"
        # It ships as a console script with a subcommand, `stegolsb steglsb`,
        # and its module path is not a usable entry point. The script sits
        # beside the interpreter that installed it, which is how it is found
        # inside a virtual environment that is not on PATH.
        beside = pathlib.Path(self.python).parent / "stegolsb"
        self.binary = str(beside) if beside.is_file() else (
            shutil.which("stegolsb") or "stegolsb"
        )

    @property
    def id(self) -> str:
        return "stego_lsb"

    def available(self) -> bool:
        if not pathlib.Path(self.binary).is_file() and not shutil.which(self.binary):
            return False
        result = subprocess.run([self.binary, "--version"],
                                capture_output=True, check=False)
        return result.returncode == 0

    def capacity(self, cover: pathlib.Path) -> int:
        with Image.open(cover) as img:
            width, height = img.size
            channels = len(img.getbands())
        return max(0, (width * height * channels) // 8 - 8)

    def embed(self, cover, payload, stego, password=None):
        self._check(cover, payload)
        with temp_workdir() as tmp:
            tmp = pathlib.Path(tmp)
            message = tmp / "payload.bin"
            message.write_bytes(payload)
            stego.parent.mkdir(parents=True, exist_ok=True)
            result = run([
                self.binary, "steglsb", "--hide",
                "-i", str(cover), "-s", str(message), "-o", str(stego),
                "-n", "1",
            ])
            if result.returncode != 0 or not stego.is_file():
                raise EmbedError(
                    "stego_lsb refused: "
                    + (result.stderr or result.stdout or b"").decode(
                        "utf-8", "replace").strip()[:300]
                )
        # It deflates the secret before writing it, so the bits in the image are
        # fewer than the bytes handed over and the effective rate is lower than
        # the payload size implies. Recorded so the manifest does not imply
        # otherwise.
        return EmbedResult(stego, len(payload), self.id,
                           {"bits_per_channel": 1, "encrypted": False,
                            "payload_deflated": True})


class SteganoEmbedder(Embedder):
    """LSB through the `stegano` library, used directly rather than by subprocess.

    It is a library first and a command second, and its command interface takes
    a message as an argument rather than a file, which makes binary payloads
    awkward. Calling the library keeps the payload exact.
    """

    formats = (".png",)

    def __init__(self, python: str | None = None) -> None:
        self.python = python or "python3"

    @property
    def id(self) -> str:
        return "stegano"

    def available(self) -> bool:
        result = subprocess.run(
            [self.python, "-c", "from stegano import lsb"],
            capture_output=True, check=False)
        return result.returncode == 0

    def capacity(self, cover: pathlib.Path) -> int:
        with Image.open(cover) as img:
            width, height = img.size
        # stegano encodes text, so a binary payload is carried as hex: two
        # characters per byte, and one character per pixel triple.
        return max(0, (width * height) // 2 - 32)

    def embed(self, cover, payload, stego, password=None):
        self._check(cover, payload)
        stego.parent.mkdir(parents=True, exist_ok=True)
        script = (
            "import sys\n"
            "from stegano import lsb\n"
            "cover, out, payload_path = sys.argv[1:4]\n"
            "with open(payload_path, 'rb') as f: data = f.read()\n"
            "secret = lsb.hide(cover, data.hex())\n"
            "secret.save(out)\n"
        )
        with temp_workdir() as tmp:
            tmp = pathlib.Path(tmp)
            message = tmp / "payload.bin"
            message.write_bytes(payload)
            result = run([self.python, "-c", script, str(cover), str(stego),
                          str(message)])
        if result.returncode != 0 or not stego.is_file():
            raise EmbedError(
                "stegano refused: "
                + (result.stderr or b"").decode("utf-8", "replace").strip()[:300]
            )
        return EmbedResult(stego, len(payload), self.id,
                           {"encoding": "hex text over LSB", "encrypted": False})


class StegosuiteEmbedder(DockerTool, Embedder):
    """Java, AES encrypted, LSB spread across the whole image.

    The GUI tool a desktop user is most likely to find packaged for their
    distribution, which is the only reason it is here: it is what someone who
    is not a researcher actually ends up running.
    """

    image = "stegobench/stegosuite:pinned"
    formats = (".png", ".bmp", ".gif", ".jpg", ".jpeg")

    @property
    def id(self) -> str:
        return "stegosuite"

    def capacity(self, cover: pathlib.Path) -> int:
        with Image.open(cover) as img:
            width, height = img.size
        return max(0, (width * height * 3) // 8 - 512)

    def embed(self, cover, payload, stego, password=None):
        self._check(cover, payload)
        with temp_workdir() as tmp:
            tmp = pathlib.Path(tmp)
            s = stage(tmp, cover, payload)
            result = self.run_in(tmp, [
                "embed", "-f", s.payload, "-k", password or "stegobench", s.cover,
            ])
            # stegosuite writes <name>_embed.<ext> beside the cover rather than
            # to a path you choose.
            produced = tmp / f"{pathlib.Path(s.cover).stem}_embed{cover.suffix}"
            if result.returncode != 0 or not produced.is_file():
                raise EmbedError(
                    "stegosuite refused: "
                    + (result.stderr or result.stdout or b"").decode(
                        "utf-8", "replace").strip()[:300]
                )
            stego.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(produced, stego)
        return EmbedResult(stego, len(payload), self.id,
                           {"encrypted": "AES", "password": True})


def _parse_capacity(text: str) -> int:
    """Steghide reports capacity as a human string: "12.3 KB", "907.0 Byte"."""
    parts = text.split()
    if not parts:
        raise EmbedError(f"cannot read a capacity from {text!r}")
    try:
        value = float(parts[0])
    except ValueError as e:
        raise EmbedError(f"cannot read a capacity from {text!r}") from e
    unit = (parts[1].lower() if len(parts) > 1 else "byte").rstrip(".")
    scale = {"byte": 1, "bytes": 1, "kb": 1024, "mb": 1024 ** 2}.get(unit)
    if scale is None:
        raise EmbedError(f"unknown capacity unit in {text!r}")
    return int(value * scale)


#: Every embedder this project knows about, tier 1 first.
ALL: tuple[type[Embedder] | Embedder, ...] = (
    SteghideEmbedder, OutguessEmbedder, OpenStegoEmbedder, StegcoreEmbedder,
    HStegoEmbedder, StegoLsbEmbedder, SteganoEmbedder, StegosuiteEmbedder,
)


def build(python: str | None = None,
          stegcore: str | None = None) -> list[Embedder]:
    """One instance of each embedder, configured for this machine."""
    return [
        SteghideEmbedder(), OutguessEmbedder(), OpenStegoEmbedder(),
        StegcoreEmbedder(stegcore), HStegoEmbedder(),
        StegoLsbEmbedder(python), SteganoEmbedder(python), StegosuiteEmbedder(),
    ]


def survey(python: str | None = None, stegcore: str | None = None) -> dict:
    """Which embedders can actually run here. Printed by `python tools.py`."""
    return {e.id: e.available() for e in build(python, stegcore)}


if __name__ == "__main__":
    import sys
    state = survey(sys.executable)
    for name, ok in state.items():
        print(f"  {'ok  ' if ok else 'MISS'} {name}")
    print(json.dumps({"available": [k for k, v in state.items() if v],
                      "missing": [k for k, v in state.items() if not v]}))
