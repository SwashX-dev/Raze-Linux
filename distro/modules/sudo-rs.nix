# sudo-rs (Trifecta Tech Foundation, memory-safe sudo) is Raze's privilege-escalation
# tool. Classic sudo is disabled and never installed; `sudo` resolves to sudo-rs.
{ config, lib, pkgs, ... }:
{
  security.sudo.enable = lib.mkForce false;

  security.sudo-rs = {
    enable = true;
    # Only members of wheel may execute sudo at all (enforced by file permissions
    # on the setuid wrapper, not just by sudoers rules).
    execWheelOnly = lib.mkDefault true;
    wheelNeedsPassword = lib.mkDefault true;
    extraConfig = ''
      Defaults use_pty
      Defaults env_reset
      Defaults timestamp_timeout=5
      Defaults passwd_tries=3
    '';
  };

  assertions = [
    {
      assertion = !config.security.sudo.enable;
      message = "Raze Linux uses sudo-rs; do not enable security.sudo.";
    }
  ];
}
