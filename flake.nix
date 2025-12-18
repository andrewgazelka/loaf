{
  description = "loaf - SQLite-backed virtual filesystem for macOS using FSKit";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    zig-overlay.url = "github:mitchellh/zig-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, zig-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        zig = zig-overlay.packages.${system}.master;
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            zig
            pkgs.sqlite
          ];

          shellHook = ''
            echo "loaf dev shell"
            echo "Zig: $(zig version)"
          '';
        };

        packages.default = pkgs.stdenv.mkDerivation {
          pname = "loaf";
          version = "0.1.0";
          src = ./.;

          nativeBuildInputs = [ zig ];
          buildInputs = [ pkgs.sqlite ];

          dontConfigure = true;

          buildPhase = ''
            export HOME=$TMPDIR
            zig build -Doptimize=ReleaseFast --prefix $out
          '';

          installPhase = ''
            # Already installed by zig build --prefix
          '';
        };
      }
    );
}
