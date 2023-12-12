{ pkgs, }:

let

rbeConfigsGen = import ../local-remote-execution/rbe-configs-gen.nix {
  inherit pkgs;
};

in

pkgs.writeShellScriptBin "generate-toolchains" ''
  #!{pkgs.bash}/bin/bash
  set -xeuo pipefail

  IMAGE_TAG=$(nix eval .#lre.imageTag --raw)

  $(nix build .#lre --print-build-logs --verbose) \
    && ./result \
    | ${pkgs.skopeo}/bin/skopeo \
      copy \
      docker-archive:/dev/stdin \
      docker-daemon:native-link-toolchain:''${IMAGE_TAG}

  ${rbeConfigsGen}/bin/rbe_configs_gen \
    --toolchain_container=native-link-toolchain:''${IMAGE_TAG} \
    --exec_os=linux \
    --target_os=linux \
    --bazel_version=${pkgs.bazel.version} \
    --output_src_root=$(pwd) \
    --output_config_path=local-remote-execution/generated \
    --bazel_path=${pkgs.bazel}/bin/bazel \
    --cpp_env_json=local-remote-execution/cpp_env.json

  pre-commit run -a
''
