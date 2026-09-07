{
  description = "ethpayserver — self-hosted EVM payment processor";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    flake-utils.url = "github:numtide/flake-utils";

    # payserver-commons source for the sandboxed build. The Nix build has no
    # network, so cargo cannot fetch the revision Cargo.toml pins - this input
    # supplies that source, and the src derivation below ASSERTS the two agree
    # rather than letting them drift into two answers.
    #
    # Moving the pin is therefore two steps, and the assert makes forgetting the
    # second one a build failure instead of a silently different binary:
    #   scripts/commons.sh pin <sha>
    #   nix flake lock --update-input payserver-commons
    #
    # Override locally with:
    #   --override-input payserver-commons path:../payserver-commons
    payserver-commons = {
      url = "git+https://github.com/randomcash/payserver-commons.git";
      flake = false;
    };
  };

  outputs = {
    self,
    nixpkgs,
    crane,
    fenix,
    flake-utils,
    payserver-commons,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        lib = pkgs.lib;

        # Rust toolchain pinned via rust-toolchain.toml, resolved by fenix.
        toolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-zC8E38iDVJ1oPIzCqTk/Ujo9+9kx9dXq7wAwPMpkpg0=";
        };

        # Extended toolchain with wasm32 target for the Leptos client.
        wasmToolchain = fenix.packages.${system}.combine [
          toolchain
          fenix.packages.${system}.targets.wasm32-unknown-unknown.latest.rust-std
        ];

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
        craneLibWasm = (crane.mkLib pkgs).overrideToolchain wasmToolchain;

        # Combined source tree: ethpayserver at the root with payserver-commons
        # embedded as a subdirectory, and a [patch] APPENDED to point the pinned
        # git dependency at it.
        #
        # This used to rewrite an existing sibling-path [patch] with
        # `--replace-fail`. RCS-218 deleted that block - commons is now pinned by
        # revision - so those patterns match nothing and `--replace-fail` aborts
        # the derivation. Appending is also the right shape now: the manifest
        # states the revision, and this redirects it to the copy Nix already has,
        # because the sandbox has no network to fetch it with.
        src = let
          ethClean = craneLib.cleanCargoSource ./.;
        in
          pkgs.runCommand "ethpayserver-src" {} ''
            cp -rL ${ethClean} $out
            chmod -R u+w $out

            cp -rL ${payserver-commons} $out/payserver-commons
            chmod -R u+w $out/payserver-commons

            # Two pins, one truth. If the flake input and the manifest disagree,
            # this build would silently compile different commons than CI and
            # every developer - so it stops here instead.
            pinned=$(sed -n 's/.*rev = "\([0-9a-f]\{40\}\)".*/\1/p' $out/Cargo.toml | head -1)
            if [ -z "$pinned" ]; then
              echo "no payserver-commons revision pinned in Cargo.toml" >&2
              exit 1
            fi
            if [ "$pinned" != "${payserver-commons.rev}" ]; then
              echo "payserver-commons pin mismatch:" >&2
              echo "  Cargo.toml  : $pinned" >&2
              echo "  flake input : ${payserver-commons.rev}" >&2
              echo "run: nix flake lock --update-input payserver-commons" >&2
              exit 1
            fi

            cat >> $out/Cargo.toml <<'EOF'

# Appended by flake.nix. The sandbox cannot fetch the pinned revision, so the
# flake input above supplies it and this redirects the dependency to that copy.
# The revision is asserted to match before this is written.
[patch."https://github.com/randomcash/payserver-commons.git"]
types = { path = "./payserver-commons/types" }
auth = { path = "./payserver-commons/auth" }
crypto = { path = "./payserver-commons/crypto" }
rates = { path = "./payserver-commons/rates" }
ui-kit = { path = "./payserver-commons/ui-kit" }
EOF
          '';

        buildInputs =
          [pkgs.openssl]
          ++ lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.Security
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];

        nativeBuildInputs =
          [pkgs.pkg-config]
          ++ lib.optionals pkgs.stdenv.isLinux [pkgs.mold];

        commonArgs = {
          inherit src buildInputs nativeBuildInputs;
          strictDeps = true;
          RUSTFLAGS = lib.optionalString pkgs.stdenv.isLinux "-C link-arg=-fuse-ld=mold";
        };

        # Pre-build workspace dependencies (cached across rebuilds).
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Helper: build a single binary crate from the workspace.
        mkBin = {
          bin,
          extraArgs ? "",
        }:
          craneLib.buildPackage (commonArgs
            // {
              inherit cargoArtifacts;
              cargoExtraArgs = "--bin ${bin} ${extraArgs}";
              doCheck = false; # tests run in checks.nextest
            });
      in {
        packages = {
          ethpayserver = mkBin {bin = "ethpayserver";};
          migrate-postgres = mkBin {bin = "migrate_postgres";};
          evmmonitor = mkBin {
            bin = "evmmonitor";
            extraArgs = "--features monitor-bin";
          };
          ethpay-mcp = mkBin {bin = "ethpay-mcp";};

          client = craneLibWasm.buildTrunkPackage {
            inherit src;
            strictDeps = true;
            trunkIndexPath = "client/index.html";
          };

          default = self.packages.${system}.ethpayserver;
        };

        checks = {
          clippy = craneLib.cargoClippy (commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--workspace --all-targets -- -D warnings";
            });

          fmt = craneLib.cargoFmt {inherit src;};

          nextest = craneLib.cargoNextest (commonArgs
            // {
              inherit cargoArtifacts;
              cargoNextestExtraArgs = "--workspace --lib";
            });
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};
          packages = with pkgs; [
            cargo-nextest
            trunk
          ];
          inputsFrom = [self.packages.${system}.ethpayserver];
        };
      }
    );
}
