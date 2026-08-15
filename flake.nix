{
  description = "UniFFI bindings generator and runtime for Haskell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
    in
    {
      devShells = nixpkgs.lib.genAttrs systems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.ghc
              pkgs.cabal-install
              pkgs.rustc
              pkgs.cargo
              pkgs.clang
              pkgs.pkg-config
            ]
            ++ pkgs.lib.optional (pkgs ? rustfmt) pkgs.rustfmt
            ++ pkgs.lib.optional (pkgs ? clippy) pkgs.clippy;
          };
        }
      );
    };
}
