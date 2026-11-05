{
  description = "Raze Linux — Rust-Powered. Performance-Driven. Minimal by Design.";

  inputs = {
    # Pinned via flake.lock. Bump deliberately with `nix flake update`.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      version = "1.1.0";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ self.overlays.default ];
      };
      lib = nixpkgs.lib;

      mkSystem = modules: lib.nixosSystem {
        inherit system;
        specialArgs = { razeVersion = version; razeSelf = self; };
        modules = [
          { nixpkgs.overlays = [ self.overlays.default ]; }
          self.nixosModules.raze
        ] ++ modules;
      };
    in
    {
      overlays.default = final: prev: import ./nix/overlay.nix { inherit version; } final prev;

      packages.${system} = {
        inherit (pkgs) r6 rix rift rift-shell raze-calamares-extensions raze-branding raze-gpu-check;
        default = pkgs.r6;

        # Raze Core: kernel + initramfs booted with r6 as the genuine PID 1.
        core-image = pkgs.callPackage ./distro/core { };
        core-image-minimal-kernel = pkgs.callPackage ./distro/core { kernel = pkgs.callPackage ./kernel/minimal.nix { }; };
        kernel-minimal = pkgs.callPackage ./kernel/minimal.nix { };

        iso = self.nixosConfigurations.raze-live.config.system.build.isoImage;
      };

      nixosModules.raze = import ./distro/modules;

      # Used by installed systems' /etc/nixos/flake.nix.
      lib.mkSystem = { modules }: mkSystem modules;

      nixosConfigurations = {
        # Live installer ISO (Calamares).
        raze-live = mkSystem [ ./distro/iso/live.nix ];
        # Reference installed system used by VM tests and as a manual-install template.
        raze-desktop = mkSystem [ ./distro/configurations/vm-desktop.nix ];
      };

      checks.${system} = import ./tests/vm { inherit pkgs self lib; };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          cargo rustc clippy rustfmt rust-analyzer
          qemu_kvm OVMF.fd python3 hyperfine s6 jq
          # Rift (Smithay) build dependencies
          pkg-config libinput seatd udev libgbm mesa libglvnd libxkbcommon wayland pixman libdrm
        ];
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [ libglvnd libxkbcommon wayland ]);
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };

      formatter.${system} = pkgs.nixpkgs-fmt;
    };
}
