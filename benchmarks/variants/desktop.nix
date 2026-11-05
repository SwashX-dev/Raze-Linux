# The reference desktop (distro/configurations/vm-desktop.nix) with a chosen desktop
# and PID 1, logged in automatically, for the benchmarks. Build e.g.:
#   nix build --impure -f benchmarks/variants/desktop.nix --argstr desktop niri
{ desktop ? "rift", init ? "r6" }:
let
  flake = builtins.getFlake (toString ../..);
  system = flake.nixosConfigurations.raze-desktop.extendModules {
    modules = [ {
      raze.desktop = desktop;
      raze.init = init;
      raze.autologinUser = "raze";
      services.qemuGuest.enable = init == "systemd";
      virtualisation.vmVariant.virtualisation.memorySize = 4096;
    } ];
  };
in
system.config.system.build.vm
