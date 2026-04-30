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

    # payserver-commons source; override locally with:
    #   --override-input payserver-commons path:../payserver-commons
    payserver-commons = {
      url = "git+https://gitlab.com/random.cash/payserver-commons.git";
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
        # embedded as a subdirectory.  The workspace [patch] section normally
        # references ../payserver-commons/* — we rewrite those to
        # ./payserver-commons/* so the entire dependency graph lives under one
        # Nix store path.
        src = let
          ethClean = craneLib.cleanCargoSource ./.;
        in
          pkgs.runCommand "ethpayserver-src" {} ''
            cp -rL ${ethClean} $out
            chmod -R u+w $out

            cp -rL ${payserver-commons} $out/payserver-commons
            chmod -R u+w $out/payserver-commons

            # Rewrite workspace [patch] paths
            substituteInPlace $out/Cargo.toml \
              --replace-fail '"../payserver-commons/' '"./payserver-commons/'

            # Rewrite client direct path deps
            substituteInPlace $out/client/Cargo.toml \
              --replace-fail '"../../payserver-commons/' '"../payserver-commons/'
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
