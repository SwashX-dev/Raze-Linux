# The reference desktop (distro/configurations/vm-desktop.nix) with systemd as PID 1,
# for benchmarks/desktop-init.py. Build:
#   nix build --impure -f benchmarks/variants/desktop-systemd.nix
let
  flake = builtins.getFlake (toString ../..);
  system = flake.nixosConfigurations.raze-desktop.extendModules {
    modules = [ {
      raze.init = "systemd";
      # The r6 qemu-ga service of the VM variant does not run under systemd.
      services.qemuGuest.enable = true;
    } ];
  };
in
system.config.system.build.vm
