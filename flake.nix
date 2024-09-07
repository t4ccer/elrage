{
  description = "elrage";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    pre-commit-hooks-nix = {
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs-stable.follows = "nixpkgs";
    };
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = inputs @ { self, ... }:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        inputs.pre-commit-hooks-nix.flakeModule
      ];

      systems = [
        "aarch64-linux"
        "i686-linux"
        "x86_64-linux"
      ];

      perSystem =
        { config
        , self'
        , inputs'
        , pkgs
        , system
        , ...
        }:
        let
          rustToolchain = pkgs.rust-bin.fromRustupToolchain {
            channel = "stable";
            components = [ "rust-analyzer" "rust-src" "rustfmt" "rustc" "cargo" ];
          };
        in
        {
          _module.args.pkgs = import self.inputs.nixpkgs {
            inherit system;
            overlays = [
              inputs.rust-overlay.overlays.rust-overlay
            ];
          };

          pre-commit.settings = {
            src = ./.;
            hooks = {
              nixpkgs-fmt.enable = true;
              rustfmt.enable = true;
            };
          };

          packages.elrage = pkgs.rustPlatform.buildRustPackage {
            name = "elrage";

            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [
              pkgs.clang
            ];

            env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";

            postInstall = ''
              mv $out/lib/libelrage.so $out/lib/elrage.so
            '';
          };

          devShells.default = pkgs.mkShell {
            shellHook = ''
              ${config.pre-commit.installationScript}
            '';

            env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";

            nativeBuildInputs = [
              pkgs.clang
              rustToolchain
              pkgs.cargo-expand
            ];
          };
        };
    };
}
