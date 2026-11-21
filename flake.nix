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
              (lib.fileset.maybeMissing ./tests/install.rs);
          };
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [
            pkgs.installShellFiles
            pkgs.makeWrapper
          ];
          postInstall = ''
            installShellCompletion --cmd outlay \
              --bash <($out/bin/outlay completions bash) \
              --zsh <($out/bin/outlay completions zsh) \
              --fish <($out/bin/outlay completions fish)
            mkdir -p $out/lib/systemd/user
            $out/bin/outlay watch --print-unit > $out/lib/systemd/user/outlay.service
          '';
          # --suffix, so a user's own xrandr still wins.
          postFixup = ''
            wrapProgram $out/bin/outlay --suffix PATH : ${lib.makeBinPath [pkgs.xrandr]}
          '';
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

    nixosModules.default = {
      config,
      lib,
      pkgs,
      ...
    }: let
      cfg = config.services.outlay;
    in {
      options.services.outlay = {
        enable = lib.mkEnableOption "outlay watch, which applies the saved monitor layout that fits when displays change";
        package = lib.mkOption {
          type = lib.types.package;
          default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          defaultText = lib.literalExpression "outlay.packages.\${pkgs.stdenv.hostPlatform.system}.default";
          description = "The outlay package to run.";
        };
      };
      config = lib.mkIf cfg.enable {
        systemd.packages = [cfg.package];
        systemd.user.services.outlay = {
          wantedBy = ["graphical-session.target"];
          # Otherwise NixOS sets PATH to coreutils & co., hiding the session's, and post_apply hooks fail.
          enableDefaultPath = false;
        };
      };
    };

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        inputsFrom = [self.packages.${pkgs.stdenv.hostPlatform.system}.default];
        packages = with pkgs; [
          clippy
          rustfmt
          rust-analyzer
          python3
          shellcheck
          sway
          wlr-randr
        ];
      };
    });
  };
}
