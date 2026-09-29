{
  description = "Slate: an AI-native Linux where humans and agents share the same desktop";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        slate = pkgs.rustPlatform.buildRustPackage {
          pname = "slate";
          version = "0.0.14";
          src = pkgs.lib.cleanSourceWith {
            src = self;
            filter = path: type:
              let base = baseNameOf path; in
              !(base == "target" || base == "docs" || base == "skills" || base == ".github" || base == "tests");
          };
          cargoLock.lockFile = ./Cargo.lock;
          postInstall = "mkdir -p $out/share/slate && cp -r ${./skills/base} $out/share/slate/skills";
          # Pure Rust; wayland-client uses its Rust backend, so no libwayland is needed.
          doCheck = true;
          # The pty tests need /bin/sh and a tty-less environment; both are fine in the sandbox.
          meta = with pkgs.lib; {
            description = "Slate agent runtime: slash, slated, slate, slate-desktop";
            homepage = "https://github.com/BenchGrid-dev/slate";
            license = licenses.gpl3Plus;
            mainProgram = "slash";
            platforms = platforms.linux;
          };
        };
        default = slate;
        iso = self.nixosConfigurations."iso-${pkgs.stdenv.hostPlatform.system}".config.system.build.isoImage;
      });

      nixosModules.default = import ./distro/nixos/module.nix { inherit self; };
      nixosModules.slate = self.nixosModules.default;

      # The live / installer image: `nix build .#iso` (for the host's architecture) or
      # `nix build .#nixosConfigurations.iso-aarch64-linux.config.system.build.isoImage`.
      nixosConfigurations = nixpkgs.lib.genAttrs (map (s: "iso-${s}") systems) (name:
        let system = nixpkgs.lib.removePrefix "iso-" name; in
        nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [ (import ./distro/iso/live.nix { inherit self nixpkgs; }) ];
        });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ rustup gcc pkg-config sway foot grim wl-clipboard python3 ];
        };
      });
    };
}
