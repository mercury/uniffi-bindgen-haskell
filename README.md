# uniffi-haskell

Haskell bindings for [UniFFI](https://mozilla.github.io/uniffi-rs) 0.32.

The Rust crate must build both a shared library, used to read UniFFI metadata, and a static library, used when linking Haskell:

```toml
[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
uniffi = "=0.32.0"
```

This library is largely "vibecoded" under careful human review.

## Nix flake
This is the most straightforward and foolproof way to use this in a polyglot project, if you are already using Nix.

### How
Add this repository as a flake input, then generate a Cabal package with:

```nix
bindings = uniffi-haskell.lib.mkUniffiHaskellCabalPackage {
  inherit pkgs;
  crate = myRustCrate;
  libraryName = "my_crate";
  packageName = "my-crate-bindings";
  license = "MPL-2.0";
};
```

## Cabal package
We support manually building a Cabal package for easy consumption in a typical Haskell project. Integrating this into a CI pipeline is left as an exercise for the reader.

### How
Create `cabal-header.txt`:

```cabal
cabal-version: 3.0
name: my-crate-bindings
version: 0.1.0
build-type: Simple
```

Generate the package:

```sh
cargo run -p uniffi-bindgen-haskell --bin uniffi-bindgen-haskell -- \
  --library target/release/libmy_crate.dylib \
  --out-dir generated \
  --cabal-file my-crate-bindings.cabal \
  --cabal-header cabal-header.txt \
  --cabal-native-library my_crate \
  --cabal-extra-lib-dir /absolute/path/to/generated/native-static

mkdir -p generated/native-static
cp target/release/libmy_crate.a generated/native-static/
```

Add both packages to `cabal.project`:

```cabal
packages:
  haskell/uniffi-runtime
  generated
```

Then run `cabal build all`.

## Raw Haskell source
If you want to integrate this into a non-Cabal build system, that is also feasible. This offers straightforward generation of just the Haskell source files and C shims.

### How
Build the Rust crate, then run:

```sh
cargo run -p uniffi-bindgen-haskell --bin uniffi-bindgen-haskell -- \
  --library target/release/libmy_crate.dylib \
  --out-dir generated
```

Use `libmy_crate.so` on Linux. The output contains Haskell sources under `generated/haskell`, C sources under `generated/cbits`, and `generated/manifest.json`. Compile the Haskell and C sources, depend on `haskell/uniffi-runtime`, and link `libmy_crate.a`.


`myRustCrate` must expose `lib/libmy_crate.dylib` or `.so` and `lib/libmy_crate.a`. Use `metadataLibrary` and `staticLibrary` to override those paths. The result is a generated Cabal source package containing the bindings and static library.
