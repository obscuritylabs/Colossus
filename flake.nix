{
  description = "Auditable runtime for agent work and durable automation";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs = { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      version = "0.11.7";
      releases = {
        aarch64-darwin = {
          target = "aarch64-apple-darwin";
          sha256 = "c33f8fae21d8dba9531a939c2468c9d696ffa64a92c5ee2665c47dce503b31d5";
        };
        x86_64-darwin = {
          target = "x86_64-apple-darwin";
          sha256 = "ad36dd071552501a565d6abfd3d38a2b40ec9c0d9be667b71c1811a55e650b05";
        };
        aarch64-linux = {
          target = "aarch64-unknown-linux-musl";
          sha256 = "df6ac90c28aa11582def5be32ce097660a47b2eb3e785771d2418457bc5f96ad";
        };
        x86_64-linux = {
          target = "x86_64-unknown-linux-musl";
          sha256 = "59b92c902e4503ac4835b952edd126f47ae1c467ec89229f3b9a11f6e026391e";
        };
      };
    in {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          release = releases.${system};
          archive = "colossus-${version}-${release.target}.tar.gz";
          package = pkgs.stdenvNoCC.mkDerivation {
            pname = "colossus";
            inherit version;
            src = pkgs.fetchurl {
              url = "https://github.com/obscuritylabs/Colossus/releases/download/v${version}/${archive}";
              inherit (release) sha256;
            };
            sourceRoot = "colossus-${version}-${release.target}";
            nativeBuildInputs = [ pkgs.makeWrapper ];
            dontConfigure = true;
            dontBuild = true;
            installPhase = ''
              runHook preInstall
              install -Dm755 colossus "$out/libexec/colossus"
              if test -f tools/rg; then
                install -Dm755 tools/rg "$out/libexec/rg"
                for notice in COPYING LICENSE-MIT UNLICENSE; do
                  install -Dm644 "tools/$notice" "$out/share/licenses/colossus/ripgrep/$notice"
                done
                bundled_rg=1
              else
                bundled_rg=0
              fi
              makeWrapper "$out/libexec/colossus" "$out/bin/colossus" \
                --set COLOSSUS_INSTALLER_KIND nix \
                --set COLOSSUS_BUNDLED_RIPGREP "$bundled_rg"
              runHook postInstall
            '';
            doInstallCheck = true;
            installCheckPhase = ''
              test "$("$out/bin/colossus" --version)" = "colossus ${version}"
              if test -x "$out/libexec/rg"; then
                "$out/libexec/rg" --version | grep '^ripgrep 15.2.0'
              fi
            '';
            meta = {
              description = "Auditable runtime for agent work and durable automation";
              homepage = "https://github.com/obscuritylabs/Colossus";
              license = pkgs.lib.licenses.asl20;
              mainProgram = "colossus";
              platforms = systems;
              sourceProvenance = [ pkgs.lib.sourceTypes.binaryNativeCode ];
            };
          };
        in {
          colossus = package;
          default = package;
        });
    };
}
