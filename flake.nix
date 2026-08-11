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
      version = "0.10.5";
      releases = {
        aarch64-darwin = {
          target = "aarch64-apple-darwin";
          sha256 = "115aaa6dffb1647b3e6e00757b5b136eb710975bad04ac72409e3ed60f012856";
        };
        x86_64-darwin = {
          target = "x86_64-apple-darwin";
          sha256 = "6d768d4204bf8854eb7edfc4c67a528501b0c6d75f4e586f3186b0e36dcbc9dd";
        };
        aarch64-linux = {
          target = "aarch64-unknown-linux-musl";
          sha256 = "30102e968e22c08057609f1e669bf96eda4df265f7407377f4dd0f7b8d05561e";
        };
        x86_64-linux = {
          target = "x86_64-unknown-linux-musl";
          sha256 = "332a39f87afec7daf68d260086c8b9a347ba48c961a32db0aed9245b9ca3e898";
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
