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
      version = "0.11.0";
      releases = {
        aarch64-darwin = {
          target = "aarch64-apple-darwin";
          sha256 = "c5140fa28173641838069e4ec331ef3848c8f3a4f3149b174e09380719e75b9a";
        };
        x86_64-darwin = {
          target = "x86_64-apple-darwin";
          sha256 = "c274017f83a11b47123e80471417548029123b245ae04b0ae4386b78c2b07478";
        };
        aarch64-linux = {
          target = "aarch64-unknown-linux-musl";
          sha256 = "efad03b81b04b66900acd4c835031aca4c2be4294a409d0c23df643e0f277407";
        };
        x86_64-linux = {
          target = "x86_64-unknown-linux-musl";
          sha256 = "f520245f4c02e5c3fe7b3e926bb2663d39b97566f0eefa34ef860a09d515f9c5";
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
