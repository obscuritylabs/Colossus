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
      version = "0.10.7";
      releases = {
        aarch64-darwin = {
          target = "aarch64-apple-darwin";
          sha256 = "035d66323a2d07026266838a0799077e16e16e1f3d8d1f803a9f5f81ee2b7d5a";
        };
        x86_64-darwin = {
          target = "x86_64-apple-darwin";
          sha256 = "f5ccbce846efe485e411dd9dd7bc878d847719fc6012e001f84ea8c6142b68cc";
        };
        aarch64-linux = {
          target = "aarch64-unknown-linux-musl";
          sha256 = "c226ce55c3a99ac4a1037b013e378b2c0a966e87bd114760fa26fab96a80eef8";
        };
        x86_64-linux = {
          target = "x86_64-unknown-linux-musl";
          sha256 = "4d357af882825d0e033a22f0ec0f914d49fbb6bf34f8a8df2721da3d80841aae";
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
              makeWrapper "$out/libexec/colossus" "$out/bin/colossus" \
                --set COLOSSUS_INSTALLER_KIND nix
              runHook postInstall
            '';
            doInstallCheck = true;
            installCheckPhase = ''
              test "$("$out/bin/colossus" --version)" = "colossus ${version}"
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
