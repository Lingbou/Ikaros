{
  description = "Ikaros local agent runtime";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ rust-overlay.overlays.default ];
        pkgs = import nixpkgs { inherit system overlays; };
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rustfmt" "clippy" ];
        };
        nativeBuildInputs = with pkgs; [
          pkg-config
          rust
        ];
        buildInputs = with pkgs; [
          openssl
        ];
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "ikaros";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          inherit nativeBuildInputs buildInputs;
          doCheck = false;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
          exePath = "/bin/ikaros";
        };

        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs buildInputs;
          packages = with pkgs; [
            cargo-deny
            cargo-audit
            docker
            git
            sqlite
          ];
          RUST_BACKTRACE = "1";
        };
      });
}
