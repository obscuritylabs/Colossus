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
      version = "0.10.9";
      releases = {
        aarch64-darwin = {
          target = "aarch64-apple-darwin";
          sha256 = "9eb7f10e7cd345c20b3c63b51484d3749bd76ae2bd470fa6756781783f6742d9";
        };
        x86_64-darwin = {
          target = "x86_64-apple-darwin";
          sha256 = "5b9a57ccd2fdc5efe8c41b5e24594daacf9dfafa38bce3a3f240f7a07dd272fa";
        };
        aarch64-linux = {
          target = "aarch64-unknown-linux-musl";
          sha256 = "a1b7d0c3501615b016d27a24d069a17b383dbb923654f65c0c83b3cbd5fd7fe9";
        };
        x86_64-linux = {
          target = "x86_64-unknown-linux-musl";
          sha256 = "19d7e1e19f2bbfecfef486294f89f9802aec9a0447857119ffa855513e93250d";
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
