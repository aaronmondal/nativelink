{
  description = "native-link";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    pre-commit-hooks = {
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs @ { self, flake-parts, crane, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-linux" ];
      imports = [ inputs.pre-commit-hooks.flakeModule ];
      perSystem = { config, pkgs, system, ... }:
        let
          customStdenv = import ./tools/llvmStdenv.nix { inherit pkgs; };

          craneLib = crane.lib.${system};

          src = pkgs.lib.cleanSourceWith {
            src = craneLib.path ./.;
            filter = path: type:
              (builtins.match "^.+/data/SekienAkashita\\.jpg" path != null) ||
              (craneLib.filterCargoSources path type);
          };

          commonArgs = {
            inherit src;
            strictDeps = true;
            buildInputs = [ ];
            nativeBuildInputs = [ pkgs.autoPatchelfHook pkgs.cacert ];
            stdenv = customStdenv;
          };

          # Additional target for external dependencies to simplify caching.
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          native-link = craneLib.buildPackage (commonArgs
            // {
            inherit cargoArtifacts;
          });

          hooks = import ./tools/pre-commit-hooks.nix { inherit pkgs; };

          publish-ghcr = import ./tools/publish-ghcr.nix { inherit pkgs; };

          local-image-test = import ./tools/local-image-test.nix { inherit pkgs; };
        in
        {
          apps = {
            default = {
              type = "app";
              program = "${native-link}/bin/cas";
            };
          };
          packages = {
            inherit publish-ghcr local-image-test;
            default = native-link;
            image = pkgs.dockerTools.streamLayeredImage {
              name = "native-link";
              contents = [
                native-link
                pkgs.dockerTools.caCertificates
              ];
              config = {
                Entrypoint = [ "/bin/cas" ];
                Labels = {
                  "org.opencontainers.image.description" = "An RBE compatible, high-performance cache and remote executor.";
                  "org.opencontainers.image.documentation" = "https://github.com/TraceMachina/native-link";
                  "org.opencontainers.image.licenses" = "Apache-2.0";
                  "org.opencontainers.image.revision" = "${self.rev or self.dirtyRev or "dirty"}";
                  "org.opencontainers.image.source" = "https://github.com/TraceMachina/native-link";
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
          pre-commit.settings = { inherit hooks; };
          devShells.default = pkgs.mkShell {
            nativeBuildInputs = [
              # Development tooling goes here.
              pkgs.cargo
              pkgs.rustc
              pkgs.pre-commit
              pkgs.bazel
              pkgs.awscli2
              pkgs.skopeo
              pkgs.dive
              pkgs.cosign

              # Additional tools from within our development environment.
              local-image-test
            ];
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
