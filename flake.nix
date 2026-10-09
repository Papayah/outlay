{
  description = "Keyboard-driven monitor layout editor for X11 and wlroots Wayland compositors";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    forAllSystems = f:
      nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
      ] (system: f nixpkgs.legacyPackages.${system});
  in {
    packages = forAllSystems (
      pkgs: let
        lib = pkgs.lib;
        outlay = pkgs.rustPlatform.buildRustPackage {
          pname = "outlay";
          version = (lib.importTOML ./Cargo.toml).package.version;
          src = lib.fileset.toSource {
            root = ./.;
            fileset =
              lib.fileset.difference (lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./tests
              ])
              ./tests/install.rs;
          };
          cargoLock.lockFile = ./Cargo.lock;
          meta = {
            description = "Keyboard-driven monitor layout editor for X11 and wlroots Wayland compositors";
            homepage = "https://github.com/Papayah/outlay";
            license = lib.licenses.gpl3Plus;
            mainProgram = "outlay";
            platforms = lib.platforms.linux;
          };
        };
      in {
        inherit outlay;
        default = outlay;
      }
    );

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        inputsFrom = [self.packages.${pkgs.stdenv.hostPlatform.system}.default];
        packages = with pkgs; [
          clippy
          rustfmt
          rust-analyzer
        ];
      };
    });
  };
}
