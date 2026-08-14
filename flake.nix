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
      version = "0.10.8";
      releases = {
        aarch64-darwin = {
          target = "aarch64-apple-darwin";
          sha256 = "be2f6b6f2274a7e1b4db4b9856d1c4627f938b6afc77cab2ffc344bb5bef39b0";
        };
        x86_64-darwin = {
          target = "x86_64-apple-darwin";
          sha256 = "6d6b5897bbd87afe9646dac8b34bde89a6ea5a50706bd376532dc8cc9a8ba25c";
        };
        aarch64-linux = {
          target = "aarch64-unknown-linux-musl";
          sha256 = "c4f26bdeeb883f085289657b99bf41ce16a6e5529f000526018b9f821379c567";
        };
        x86_64-linux = {
          target = "x86_64-unknown-linux-musl";
          sha256 = "9451a77d0d765c563c7d7aa0bab45f92bdb16e87bc956f7a3119b4aec60f3fba";
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
