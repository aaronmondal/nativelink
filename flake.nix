{
  description = "nativelink";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    pre-commit-hooks = {
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs @ {
    self,
    flake-parts,
    crane,
    rust-overlay,
    ...
  }:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = [
        "x86_64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      imports = [inputs.pre-commit-hooks.flakeModule];
      perSystem = {
        config,
        pkgs,
        system,
        ...
      }: let
        stable-rust = pkgs.rust-bin.stable."1.75.0";
        nightly-rust = pkgs.rust-bin.nightly."2024-01-01";

        maybeDarwinDeps = pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.CF
          pkgs.darwin.apple_sdk.frameworks.Security
          pkgs.libiconv
        ];

        customStdenv = import ./tools/llvmStdenv.nix {inherit pkgs;};

        # TODO(aaronmondal): This doesn't work with rules_rust yet.
        # Tracked in https://github.com/TraceMachina/nativelink/issues/477.
        customClang = pkgs.callPackage ./tools/customClang.nix {
          inherit pkgs;
          stdenv = customStdenv;
        };

        craneLib =
          if pkgs.stdenv.isDarwin
          then crane.lib.${system}
          else
            (crane.mkLib pkgs).overrideToolchain (pkgs.rust-bin.stable.latest.default.override {
              targets = ["x86_64-unknown-linux-musl"];
            });

        src = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter = path: type:
            (builtins.match "^.+/data/SekienAkashita\\.jpg" path != null)
            || (craneLib.filterCargoSources path type);
        };

        commonArgs =
          {
            inherit src;
            stdenv =
              if pkgs.stdenv.isDarwin
              then customStdenv
              else pkgs.pkgsMusl.stdenv;
            strictDeps = true;
            buildInputs = maybeDarwinDeps;
            nativeBuildInputs = [pkgs.cacert] ++ maybeDarwinDeps;
          }
          // pkgs.lib.optionalAttrs (!pkgs.stdenv.isDarwin) {
            CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
            CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
          };

        # Additional target for external dependencies to simplify caching.
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        nativelink = craneLib.buildPackage (commonArgs
          // {
            inherit cargoArtifacts;
          });

        hooks = import ./tools/pre-commit-hooks.nix {inherit pkgs;};

        publish-ghcr = import ./tools/publish-ghcr.nix {inherit pkgs;};

        local-image-test = import ./tools/local-image-test.nix {inherit pkgs;};

        generate-toolchains = import ./tools/generate-toolchains.nix {inherit pkgs;};
      in {
        _module.args.pkgs = import self.inputs.nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };
        apps = {
          default = {
            type = "app";
            program = "${nativelink}/bin/nativelink";
          };
        };
        packages = {
          inherit publish-ghcr local-image-test;
          default = nativelink;
          lre = import ./local-remote-execution/image.nix {inherit pkgs nativelink;};
          image = pkgs.dockerTools.streamLayeredImage {
            name = "nativelink";
            contents = [
              nativelink
              pkgs.dockerTools.caCertificates
            ];
            config = {
              Entrypoint = ["/bin/nativelink"];
              Labels = {
                "org.opencontainers.image.description" = "An RBE compatible, high-performance cache and remote executor.";
                "org.opencontainers.image.documentation" = "https://github.com/TraceMachina/nativelink";
                "org.opencontainers.image.licenses" = "Apache-2.0";
                "org.opencontainers.image.revision" = "${self.rev or self.dirtyRev or "dirty"}";
                "org.opencontainers.image.source" = "https://github.com/TraceMachina/nativelink";
                "org.opencontainers.image.title" = "Native Link";
                "org.opencontainers.image.vendor" = "Trace Machina, Inc.";
              };
            };
          };
        };
        checks = {
          # TODO(aaronmondal): Fix the tests.
          # tests = craneLib.cargoNextest (commonArgs
          #   // {
          #   inherit cargoArtifacts;
          #   cargoNextestExtraArgs = "--all";
          #   partitions = 1;
          #   partitionType = "count";
          # });
        };
        pre-commit.settings = {
          inherit hooks;
          tools = let
            mkOverrideTools = pkgs.lib.mkOverride (pkgs.lib.modules.defaultOverridePriority - 1);
          in {
            rustfmt = mkOverrideTools nightly-rust.rustfmt;
          };
        };
        devShells.default = pkgs.mkShell {
          nativeBuildInputs =
            [
              # Development tooling goes here.
              stable-rust.default
              pkgs.pre-commit
              pkgs.bazel_7
              pkgs.awscli2
              pkgs.skopeo
              pkgs.dive
              pkgs.cosign
              pkgs.kubectl
              pkgs.kubernetes-helm
              pkgs.cilium-cli
              pkgs.yarn

              # Additional tools from within our development environment.
              local-image-test
              generate-toolchains
              customClang
            ]
            ++ maybeDarwinDeps;
          shellHook = ''
            # Generate the .pre-commit-config.yaml symlink when entering the
            # development shell.
            ${config.pre-commit.installationScript}

            # The Bazel and Cargo builds in nix require a Clang toolchain.
            # TODO(aaronmondal): The Bazel build currently uses the
            #                    irreproducible host C++ toolchain. Provide
            #                    this toolchain via nix for bitwise identical
            #                    binaries across machines.
            export CC=clang
          '';
        };
      };
    };
}
