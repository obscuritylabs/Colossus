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
      version = "0.11.10";
      releases = {
        aarch64-darwin = {
          target = "aarch64-apple-darwin";
          sha256 = "a0c5a8a5f9e75a31acd2d3ca13120cdd42a898fe755a1bc3f712aebec50e0321";
        };
        x86_64-darwin = {
          target = "x86_64-apple-darwin";
          sha256 = "3293dbdf5dbdcae1b00d5cdffa599525868a7f2609ab505e1ee7ecf3c44a09fd";
        };
        aarch64-linux = {
          target = "aarch64-unknown-linux-musl";
          sha256 = "3e5d0c329a33fda76ea495a2dd96cdb96e09f2132ca590a30eff3db03bb4af2c";
        };
        x86_64-linux = {
          target = "x86_64-unknown-linux-musl";
          sha256 = "31afc483883e180a7a820f09b46846f8b9c7c672437fc4d7947da2dc8f588b06";
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
