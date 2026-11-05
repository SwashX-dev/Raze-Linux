# Raze Linux packages, exposed to every Raze system through nixpkgs.overlays.
{ version }:
final: prev:
let
  inherit (final) lib;
  fs = lib.fileset;
  workspaceSrc = fs.toSource {
    root = ../.;
    fileset = fs.unions [ ../Cargo.toml ../Cargo.lock ../r6 ../rix ];
  };
  rustWorkspacePackage = { pname, description, nativeCheckInputs ? [ ], postInstall ? "", extra ? { } }:
    final.rustPlatform.buildRustPackage ({
      inherit pname version postInstall nativeCheckInputs;
      src = workspaceSrc;
      cargoLock.lockFile = ../Cargo.lock;
      cargoBuildFlags = [ "-p" pname ];
      cargoTestFlags = [ "-p" pname ];
      meta = {
        inherit description;
        homepage = "https://github.com/SwashX-dev/Raze-Linux";
        license = lib.licenses.mit;
        platforms = lib.platforms.linux;
      };
    } // extra);
in
{
  r6 = rustWorkspacePackage {
    pname = "r6";
    description = "Rust init and single-process service supervisor inspired by s6";
    nativeCheckInputs = [ final.procps ];
  };

  rix = rustWorkspacePackage {
    pname = "rix";
    description = "Fast Rust front-end for Nix-compatible package management";
    nativeCheckInputs = [ final.nix ];
    extra.nativeBuildInputs = [ final.makeWrapper ];
    # Rix delegates evaluation, building and substitution to Nix; make sure a Nix is
    # always reachable even outside a NixOS PATH.
    postInstall = ''
      wrapProgram $out/bin/rix --suffix PATH : ${lib.makeBinPath [ final.nix final.git ]}
    '';
  };

  rift = final.callPackage ../rift/package.nix { inherit version; };

  raze-branding = final.callPackage ../distro/branding { inherit version; };

  raze-calamares-extensions = final.callPackage ../installer/calamares { inherit version; };

  # The Raze source tree, copied onto installed systems so /etc/nixos can build from it
  # offline (same flake, same lock, same store paths as the ISO).
  raze-source = fs.toSource {
    root = ../.;
    fileset = fs.unions [
      ../flake.nix
      ../flake.lock
      ../Cargo.toml
      ../Cargo.lock
      ../nix
      ../r6
      ../rix
      ../distro
      ../installer
      ../kernel
      ../LICENSE
    ];
  };
}
