# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# A Nix flake for the Rust half, so somebody can run
#
#     nix run github:elementmerc/stegobench
#
# or add this repository as a flake input and get the same binary a build from
# source would produce.
#
# WHY IT IS THIS PLAIN
#
# This file is deliberately the dullest thing that works: one input, no
# flake-utils, no overlays, no toolchain fetcher. Every extra input is a
# dependency that runs with the builder's privileges (baseline Section 5), and
# a flake that is subtly clever is a flake nobody can debug from an error
# message. Four systems are listed literally rather than generated, because the
# list is exactly the set the release workflow builds for and it should change
# in the same commit that changes the workflow.
#
# WHAT IT DOES NOT DO
#
# It does not honour `rust-toolchain.toml`. `buildRustPackage` uses the rustc
# that comes with the pinned nixpkgs, which is the whole point of the flake
# lock, so the toolchain here is whatever that revision carries rather than
# 1.98.1. The workspace has no nightly features and no pinned-version
# dependency, so this is a difference rather than a problem; a build failure on
# a newer compiler would show up here first, which is useful.
{
  description = "A reproducible benchmark for steganalysis: run detectors and embedders as sandboxed plugins, score them against a labelled corpus, and get back a versioned JSON document that names the exact bytes it was measured on.";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      # The same four platforms the release workflow produces binaries for.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Read rather than repeated. A version written here by hand is a fifth
      # copy of a number four gates already check, and it would drift on the
      # first release nobody remembered this file during.
      workspaceVersion =
        (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
    in
    {
      packages = forAllSystems (pkgs: rec {
        stegobench = pkgs.rustPlatform.buildRustPackage {
          pname = "stegobench";
          version = workspaceVersion;

          src = self;

          # The committed lockfile, so the dependency set a Nix build resolves
          # is the one `cargo build --locked` resolves and the one CI tested.
          cargoLock.lockFile = ./Cargo.lock;

          # Only the binary crate is wanted here. The other three are built as
          # its dependencies anyway.
          cargoBuildFlags = [ "-p" "stegobench-cli" ];
          cargoTestFlags = [ "-p" "stegobench-cli" ];

          # build.rs mirrors the generated man pages to `target/man` on a
          # best-effort basis, so this is guarded rather than assumed: it must
          # not turn a missing nicety into a failed build.
          postInstall = ''
            if [ -d target/man ]; then
              for page in target/man/*.1; do
                [ -e "$page" ] || continue
                install -Dm644 "$page" "$out/share/man/man1/$(basename "$page")"
              done
            fi
          '';

          meta = with pkgs.lib; {
            description = "A reproducible benchmark for steganalysis";
            homepage = "https://github.com/elementmerc/stegobench";
            license = licenses.agpl3Plus;
            mainProgram = "stegobench";
            platforms = platforms.unix;
          };
        };

        default = stegobench;
      });
    };
}
