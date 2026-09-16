#!/usr/bin/env python3
"""The real end-user tools, behind one interface.

WHY THESE AND NOT THE ACADEMIC SCHEMES
--------------------------------------
`embed_adaptive.py` covers the cost-based family, HUGO through MiPOD, which is
what the literature measures. This module covers what people actually download
and run. The two arms answer different questions and a corpus needs both:
REVEAL (Netherlands Forensic Institute, 2025) built its reputation on 51 real
tools and excluded the adaptive schemes on purpose, and every corpus with good
adaptive coverage is JPEG-only and derived from BOSSbase.

A real tool is not a reference implementation with a nicer name. It picks its
own payload encoding, adds its own headers, encrypts by default or not at all,
and often writes a container the academic schemes never touch. Those choices are
what a deployed detector actually meets, and several of them are detectable by
structure rather than by statistics, which is a different kind of finding.

THE SHAPE OF THE INTERFACE, AND WHY IT IS THIN
----------------------------------------------
Every tool here is a subprocess, whether it runs from the virtual environment,
from a pinned Docker image, or as a native binary. So `Embedder` does four
things and refuses to do more:

    id()          what the manifest calls this arm
    available()   can it run here, right now, checked rather than assumed
    capacity()    how many payload bytes this cover can take
    embed()       cover plus payload in, stego file out, or a clear refusal

Capacity is separate from embed on purpose. Tools disagree wildly about how much
a given cover holds, and a sweep that asks for more than a tool can carry gets
either a truncated payload or a crash, both of which are silent corruption in a
corpus. Asking first turns that into a recorded skip.

WHAT IS DELIBERATELY NOT HERE
-----------------------------
Detectors. Aletheia, zsteg, StegExpose and stegoveritas are steganalysis tools,
not embedders, and they belong on the evaluation side of the benchmark. It is an
easy confusion to make because they ship beside each other in the same
comparator directory, and a corpus arm built from a detector would be nothing at
all.
"""
from __future__ import annotations

import abc
import dataclasses
import os
import pathlib
import shutil
import subprocess
import tempfile

# A tool that has not answered in this long is not going to. Real tools on real
# covers finish in under a second; the ceiling is here so one wedged subprocess
# cannot stall an overnight sweep.
DEFAULT_TIMEOUT = 120


class EmbedError(RuntimeError):
    """A refusal a caller can record: too small, wrong format, tool absent."""


@dataclasses.dataclass(frozen=True)
class EmbedResult:
    stego: pathlib.Path
    payload_bytes: int
    tool: str
    detail: dict


class Embedder(abc.ABC):
    """One steganography tool, reduced to what a corpus build needs from it."""

    #: Which cover formats this tool will accept, lowercase with the dot.
    formats: tuple[str, ...] = (".png",)

    @property
    @abc.abstractmethod
    def id(self) -> str:
        """The arm name. Goes in the manifest and into directory paths."""

    @abc.abstractmethod
    def available(self) -> bool:
        """Whether this tool can run here. Measured, never assumed."""

    @abc.abstractmethod
    def capacity(self, cover: pathlib.Path) -> int:
        """Payload bytes this cover can hold, as this tool counts them."""

    @abc.abstractmethod
    def embed(self, cover: pathlib.Path, payload: bytes,
              stego: pathlib.Path, password: str | None = None) -> EmbedResult:
        """Write `payload` into `cover`, producing `stego`."""

    def accepts(self, cover: pathlib.Path) -> bool:
        return cover.suffix.lower() in self.formats

    def _check(self, cover: pathlib.Path, payload: bytes) -> None:
        """The three refusals every tool shares, so each one need not repeat them."""
        if not self.available():
            raise EmbedError(f"{self.id} is not available on this machine")
        if not cover.is_file():
            raise EmbedError(f"no cover at {cover}")
        if not self.accepts(cover):
            raise EmbedError(
                f"{self.id} does not take {cover.suffix or 'that'} covers, "
                f"only {', '.join(self.formats)}"
            )
        if not payload:
            raise EmbedError("an empty payload is not an embedding")
        room = self.capacity(cover)
        if len(payload) > room:
            raise EmbedError(
                f"{self.id}: payload is {len(payload)} bytes and this cover "
                f"holds {room}. Ask capacity() before sweeping a rate."
            )


def run(argv: list[str], timeout: int = DEFAULT_TIMEOUT,
        stdin: bytes | None = None) -> subprocess.CompletedProcess:
    """One subprocess with a deadline, never inheriting a shell."""
    try:
        return subprocess.run(argv, capture_output=True, input=stdin,
                              timeout=timeout, check=False)
    except subprocess.TimeoutExpired as e:
        raise EmbedError(f"{argv[0]} did not finish within {timeout}s") from e
    except FileNotFoundError as e:
        raise EmbedError(f"{argv[0]} is not on PATH") from e


class DockerTool:
    """Shared plumbing for a tool that runs from a pinned image.

    The tools reached this way are packaged for a distribution rather than for a
    virtual environment, and building them on the host would mean either root or
    a pile of build dependencies. A pinned image also fixes the version, which
    matters more here than usual: REVEAL says plainly that a tool corpus "will
    be highly sensitive to software versioning", and a corpus that cannot say
    which version wrote its stego files is missing a column.
    """

    image: str = ""

    def available(self) -> bool:
        if not shutil.which("docker"):
            return False
        result = subprocess.run(
            ["docker", "image", "inspect", self.image],
            capture_output=True, check=False,
        )
        return result.returncode == 0

    def run_in(self, workdir: pathlib.Path, args: list[str],
               timeout: int = DEFAULT_TIMEOUT) -> subprocess.CompletedProcess:
        """Run the image's entrypoint over a directory mounted at /data.

        Hostile-by-default, per baseline section 5: no network, all capabilities
        dropped, and the only writable path is the work directory it was handed.
        These are third-party binaries and several are unmaintained.
        """
        return run([
            "docker", "run", "--rm",
            "--network=none",
            "--cap-drop=ALL",
            "--security-opt", "no-new-privileges",
            f"--user={os.getuid()}:{os.getgid()}",
            "-v", f"{workdir}:/data",
            "-w", "/data",
            self.image, *args,
        ], timeout=timeout)


@dataclasses.dataclass
class _Staged:
    """A temp directory holding the cover and payload a docker tool will see."""
    root: pathlib.Path
    cover: str
    payload: str
    out: str


def stage(tmp: pathlib.Path, cover: pathlib.Path, payload: bytes,
          out_suffix: str | None = None) -> _Staged:
    """Copy a cover and its payload where a container can reach them."""
    cover_name = f"cover{cover.suffix}"
    out_name = f"stego{out_suffix or cover.suffix}"
    shutil.copy2(cover, tmp / cover_name)
    (tmp / "payload.bin").write_bytes(payload)
    return _Staged(tmp, cover_name, "payload.bin", out_name)


def temp_workdir():
    """A work directory a container can write into, cleaned up afterwards."""
    return tempfile.TemporaryDirectory(prefix="stegobench-")
