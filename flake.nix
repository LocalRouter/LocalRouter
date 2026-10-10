{
  description = "LocalRouter - local OpenAI-compatible API gateway with intelligent multi-provider routing";

  # The package itself is packaging/nix/package.nix, a repackage of the
  # released Linux .deb; packaging/nix/sources.json pins the release and is
  # bumped by the release workflow. See packaging/README.md.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      overlays.default = final: _prev: {
        localrouter = final.callPackage ./packaging/nix/package.nix { };
      };

      packages = forAllSystems (pkgs: rec {
        localrouter = pkgs.callPackage ./packaging/nix/package.nix { };
        default = localrouter;
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          meta.description = "Run LocalRouter";
        };
      });

      checks = forAllSystems (pkgs: {
        localrouter = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      });
    };
}
