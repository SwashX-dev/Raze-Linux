# The reference desktop (distro/configurations/vm-desktop.nix) with a chosen desktop
# and PID 1, logged in automatically, for the benchmarks. Build e.g.:
#   nix build --impure -f benchmarks/variants/desktop.nix --argstr desktop niri
{ desktop ? "rift", init ? "r6" }:
let
  flake = builtins.getFlake (toString ../..);
  system = flake.nixosConfigurations.raze-desktop.extendModules {
    modules = [ ({ lib, ... }: {
      raze.desktop = lib.mkForce desktop;
      raze.init = init;
      raze.autologinUser = "raze";
      services.qemuGuest.enable = lib.mkIf (init == "systemd") true;
      virtualisation.vmVariant.virtualisation.memorySize = lib.mkForce 4096;
    }) ];
  };
in
system.config.system.build.vm
