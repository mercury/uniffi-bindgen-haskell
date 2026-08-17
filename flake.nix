{
  description = "UniFFI bindings generator and runtime for Haskell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs, ... }:
    let
      inherit (nixpkgs) lib;

      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];

      forAllSystems = lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        rec {
          uniffi-bindgen-haskell = pkgs.rustPlatform.buildRustPackage {
            pname = "uniffi-bindgen-haskell";
            version = "0.1.0";
            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "--package"
              "uniffi-bindgen-haskell"
              "--bin"
              "uniffi-bindgen-haskell"
            ];
            cargoTestFlags = [
              "--package"
              "uniffi-bindgen-haskell"
            ];
          };

          default = uniffi-bindgen-haskell;
        }
      );

      lib.mkUniffiHaskellCabalPackage =
        {
          pkgs,
          crate,
          libraryName ? builtins.replaceStrings [ "-" ] [ "_" ] (crate.pname or (lib.getName crate)),
          packageName ? "${builtins.replaceStrings [ "_" ] [ "-" ] libraryName}-bindings",
          version ? crate.version or "0.1.0",
          license ? "NONE",
          synopsis ? "Haskell bindings for ${libraryName}",
          extraCabalFields ? "",
          metadataLibrary ? "${crate}/lib/lib${libraryName}${pkgs.stdenv.hostPlatform.extensions.sharedLibrary}",
          staticLibrary ? "${crate}/lib/lib${libraryName}${pkgs.stdenv.hostPlatform.extensions.staticLibrary}",
        }:
        assert lib.assertMsg
          (pkgs.stdenv.buildPlatform.system == pkgs.stdenv.hostPlatform.system)
          "mkUniffiHaskellCabalPackage does not support cross compilation because the bindgen must load the crate's shared library";
        let
          system = pkgs.stdenv.hostPlatform.system;
          bindgen = self.packages.${system}.uniffi-bindgen-haskell;
          cabalHeader = pkgs.writeText "${packageName}-cabal-header" ''
            cabal-version: 3.0
            name: ${packageName}
            version: ${version}
            license: ${license}
            synopsis: ${synopsis}
            build-type: Simple
            ${extraCabalFields}
          '';
        in
        pkgs.runCommand "${packageName}-${version}" {
          nativeBuildInputs = [ bindgen ];
          passthru = {
            inherit crate libraryName packageName version;
          };
        } ''
          if [[ ! -f ${lib.escapeShellArg (toString metadataLibrary)} ]]; then
            echo "UniFFI metadata library not found: ${toString metadataLibrary}" >&2
            exit 1
          fi
          if [[ ! -f ${lib.escapeShellArg (toString staticLibrary)} ]]; then
            echo "UniFFI static library not found: ${toString staticLibrary}" >&2
            exit 1
          fi

          mkdir -p "$out/native-static"
          uniffi-bindgen-haskell \
            --library ${lib.escapeShellArg (toString metadataLibrary)} \
            --out-dir "$out" \
            --cabal-file ${lib.escapeShellArg "${packageName}.cabal"} \
            --cabal-header ${lib.escapeShellArg (toString cabalHeader)} \
            --cabal-native-library ${lib.escapeShellArg libraryName} \
            --cabal-extra-lib-dir "$out/native-static"
          install -m444 \
            ${lib.escapeShellArg (toString staticLibrary)} \
            "$out/native-static/lib${libraryName}${pkgs.stdenv.hostPlatform.extensions.staticLibrary}"
        '';

      devShells = forAllSystems (
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
