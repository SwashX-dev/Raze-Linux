# Raze Minimal kernel: x86_64_defconfig + kvm_guest.config + config/minimal-kvm.config,
# built from the same pinned Linux source as linux_latest.
{ lib, linux_latest, linuxManualConfig, stdenv, buildPackages, flex, bison, bc, perl }:
let
  base = linux_latest;
  configfile = stdenv.mkDerivation {
    pname = "linux-config-raze-minimal";
    inherit (base) version src;
    depsBuildBuild = [ buildPackages.stdenv.cc ];
    nativeBuildInputs = [ flex bison bc perl ];
    dontConfigure = true;
    buildPhase = ''
      make ARCH=x86_64 x86_64_defconfig kvm_guest.config
      KCONFIG_CONFIG=.config scripts/kconfig/merge_config.sh -m .config ${./config/minimal-kvm.config}
      make ARCH=x86_64 olddefconfig
      # Fail if a requested option did not survive dependency resolution.
      missing=0
      while IFS= read -r line; do
        case "$line" in ''|\#*) continue ;; esac
        key=''${line%%=*}; val=''${line#*=}
        if [ "$val" = n ]; then
          grep -q "^# $key is not set" .config || grep -q "^$key=n" .config || { echo "not disabled: $key"; missing=1; }
        else
          grep -qx "$key=$val" .config || { echo "not applied: $line"; missing=1; }
        fi
      done < ${./config/minimal-kvm.config}
      [ $missing = 0 ]
    '';
    installPhase = "cp .config $out";
  };
in
(linuxManualConfig {
  inherit (base) src version;
  modDirVersion = "${base.modDirVersion}-raze-minimal";
  inherit configfile;
  allowImportFromDerivation = true;
}).overrideAttrs (old: {
  passthru = (old.passthru or { }) // { inherit configfile; };
})
