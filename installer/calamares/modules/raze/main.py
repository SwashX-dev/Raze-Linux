#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
#
# Raze Linux Calamares job module.
#
# Runs after the partition and mount modules. It:
#   1. generates hardware-configuration.nix (nixos-generate-config) and detects
#      other operating systems for the boot menu (dual boot),
#   2. copies the pinned Raze source tree to /etc/nixos/raze,
#   3. writes a flake-based /etc/nixos (flake.nix + configuration.nix) from the
#      choices made in the installer,
#   4. runs nixos-install against that flake, streaming progress to Calamares,
#   5. keeps the full log at /var/log/raze-install.log on the installed system.
#
# Calamares runs as root on the Raze live ISO (started through sudo-rs), so no
# privilege helper is needed here.

import datetime
import json
import os
import re
import shutil
import subprocess

import libcalamares

RAZE_SOURCE = "/etc/raze/source"


def pretty_name():
    return "Installing Raze Linux"


_status = "Preparing installation"


def pretty_status_message():
    return _status


def set_status(msg, progress):
    global _status
    _status = msg
    libcalamares.job.setprogress(progress)


def nix_string(value):
    """Quote a Python string as a Nix string literal."""
    s = str(value).replace("\\", "\\\\").replace('"', '\\"').replace("${", "\\${")
    return '"' + s + '"'


def valid_unix_name(name):
    return re.fullmatch(r"[a-z_][a-z0-9_-]{0,31}", name or "") is not None


# EFI loaders of other systems, chainloaded from GRUB: (path on an ESP, menu title).
EFI_LOADERS = [
    ("/EFI/Microsoft/Boot/bootmgfw.efi", "Windows Boot Manager"),
]
# Directories on the ESP that belong to this installation (or are generic fallbacks).
OWN_EFI_DIRS = {"raze", "nixos-boot", "boot"}


def efi_entries(esp):
    """Other systems' boot loaders on the ESP mounted at `esp` (paths are ESP-relative,
    with forward slashes, as GRUB wants them)."""
    found = []
    base = os.path.join(esp, "EFI")
    if not os.path.isdir(base):
        return found
    names = {}
    for d in os.listdir(base):
        names[d.lower()] = d
    for path, title in EFI_LOADERS:
        parts = path.strip("/").split("/")
        if parts[1].lower() in names:
            real = os.path.join(base, names[parts[1].lower()], *parts[2:])
            if os.path.isfile(real):
                found.append(("/EFI/{}/{}".format(names[parts[1].lower()], "/".join(parts[2:])), title))
    for low, d in sorted(names.items()):
        if low in OWN_EFI_DIRS or low == "microsoft":
            continue
        for loader in ("shimx64.efi", "grubx64.efi", "systemd-bootx64.efi"):
            if os.path.isfile(os.path.join(base, d, loader)):
                title = d if low != "systemd" else "Linux (systemd-boot)"
                found.append(("/EFI/{}/{}".format(d, loader), title.capitalize() if title.islower() else title))
                break
    return found


def os_prober():
    """Other systems found by os-prober: list of (device, loader-or-None, title, kind)."""
    try:
        out = subprocess.run(["os-prober"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                             timeout=120).stdout.decode("utf-8", "replace")
    except (OSError, subprocess.TimeoutExpired):
        return []
    result = []
    for line in out.splitlines():
        fields = line.split(":")
        if len(fields) < 4:
            continue
        dev, title, _short, kind = fields[0], fields[1], fields[2], fields[3]
        loader = None
        if "@" in dev:
            dev, loader = dev.split("@", 1)
        result.append((dev, loader, title, kind))
    return result


def grub_chainload(title, path):
    path = path.replace("\\", "/")
    return (
        'menuentry "{title}" {{\n'
        "  insmod part_gpt\n"
        "  insmod fat\n"
        "  search --no-floppy --file --set=root {path}\n"
        "  chainloader {path}\n"
        "}}\n"
    ).format(title=title.replace('"', ""), path=path)


def bootloader_config(gs, root, others):
    """GRUB configuration. `others`: other systems found (dual boot)."""
    fw = gs.value("firmwareType")
    timeout = 5 if others else 2
    if fw == "efi":
        esp = os.path.join(root, "boot/efi")
        entries = efi_entries(esp)
        seen = {p.lower() for p, _ in entries}
        # Loaders on other disks' ESPs (os-prober reports them as device@path).
        for _dev, loader, title, kind in others:
            if kind == "efi" and loader and loader.lower() not in seen and \
                    loader.strip("/").split("/")[1].lower() not in OWN_EFI_DIRS:
                entries.append((loader, title))
                seen.add(loader.lower())
        conf = (
            # GRUB on EFI too: Raze avoids systemd components, including systemd-boot.
            "  boot.loader.grub.enable = true;\n"
            "  boot.loader.grub.efiSupport = true;\n"
            '  boot.loader.grub.device = "nodev";\n'
            "  boot.loader.grub.configurationLimit = 10;\n"
            "  # The EFI system partition (possibly shared with other systems); kernels\n"
            "  # stay on the root filesystem.\n"
            '  boot.loader.efi.efiSysMountPoint = "/boot/efi";\n'
            "  boot.loader.efi.canTouchEfiVariables = true;\n"
            "  boot.loader.timeout = {};\n"
        ).format(timeout)
        if entries:
            conf += "  # Other operating systems found during installation (dual boot).\n"
            conf += "  boot.loader.grub.extraEntries = ''\n"
            for path, title in entries:
                for line in grub_chainload(title, path).splitlines():
                    conf += "    " + line + "\n"
            conf += "  '';\n"
        return conf, [t for _, t in entries]
    bootloader = gs.value("bootLoader")
    device = bootloader["installPath"] if bootloader else "nodev"
    conf = (
        "  boot.loader.grub.enable = true;\n"
        "  boot.loader.grub.device = {};\n"
        "  boot.loader.grub.configurationLimit = 10;\n"
        "  boot.loader.timeout = {};\n"
    ).format(nix_string(device), timeout)
    if others:
        conf += "  # Other operating systems were found: list them in the boot menu.\n"
        conf += "  boot.loader.grub.useOSProber = true;\n"
    return conf, [t for _, _, t, _ in others]


def locale_config(gs):
    out = ""
    zone = None
    if gs.value("locationRegion") and gs.value("locationZone"):
        zone = "{}/{}".format(gs.value("locationRegion"), gs.value("locationZone"))
        out += "  time.timeZone = {};\n".format(nix_string(zone))
    conf = gs.value("localeConf") or {}
    lang = conf.get("LANG", "en_US.UTF-8").split("/")[0]
    out += "  i18n.defaultLocale = {};\n".format(nix_string(lang))
    extra = {k: v.split("/")[0] for k, v in conf.items() if k != "LANG" and v.split("/")[0] != lang}
    if extra:
        out += "  i18n.extraLocaleSettings = {\n"
        for k, v in sorted(extra.items()):
            out += "    {} = {};\n".format(k, nix_string(v))
        out += "  };\n"
    layout = gs.value("keyboardLayout")
    variant = gs.value("keyboardVariant")
    if layout:
        out += "  services.xserver.xkb.layout = {};\n".format(nix_string(layout))
        out += "  services.xserver.xkb.variant = {};\n".format(nix_string(variant or ""))
        out += "  console.useXkbConfig = true;\n"
    return out


def choice(gs, key, allowed, default):
    value = gs.value("packagechooser_" + key)
    if isinstance(value, list):
        value = value[0] if value else None
    return value if value in allowed else default


def generate_configuration(gs, root="/", others=()):
    desktop = choice(gs, "desktop", ["rift", "niri", "hyprland", "dwl", "xfce", "lxqt", "plasma", "gnome", "dwm"], "rift")
    profile = choice(gs, "profile", ["desktop", "development", "minimal"], "desktop")
    if profile == "desktop" and desktop != "rift":
        profile = "custom"
    username = gs.value("username")
    if not valid_unix_name(username):
        raise ValueError("invalid user name: {!r}".format(username))
    fullname = gs.value("fullname") or username
    hostname = gs.value("hostname") or "raze"
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]{0,62}", hostname):
        hostname = "raze"

    cfg = "# Raze Linux system configuration.\n"
    cfg += "# Generated by the Raze installer on {}.\n".format(datetime.date.today().isoformat())
    cfg += "# Apply changes with: sudo rix system switch   (or: sudo nixos-rebuild switch --flake /etc/nixos#raze)\n"
    cfg += "{ config, lib, pkgs, ... }:\n\n{\n"
    cfg += "  # Installation profile: minimal | desktop | custom | development\n"
    cfg += "  raze.profile = {};\n".format(nix_string(profile))
    cfg += "  # Desktop: rift (default) | niri | hyprland | dwl | xfce | lxqt | plasma | gnome | dwm | none\n"
    cfg += "  raze.desktop = {};\n".format(nix_string(desktop if profile != "minimal" else "none"))
    cfg += "  # Kernel profile: performance (default) | generic | legacy | minimal\n"
    cfg += '  raze.kernel.profile = "performance";\n'
    cfg += "  # Init system and service manager: r6 (default) | systemd (fallback)\n"
    # GNOME needs systemd (it is built on it); every other desktop runs on r6.
    cfg += '  raze.init = {};\n\n'.format(nix_string("systemd" if desktop == "gnome" else "r6"))
    boot, boot_others = bootloader_config(gs, root, list(others))
    cfg += boot + "\n"
    cfg += "  networking.hostName = {};\n".format(nix_string(hostname))
    cfg += locale_config(gs) + "\n"
    cfg += "  users.users.{} = {{\n".format(nix_string(username))
    cfg += "    isNormalUser = true;\n"
    cfg += "    description = {};\n".format(nix_string(fullname))
    cfg += '    extraGroups = [ "wheel" "networkmanager" "video" "audio" ];\n'
    cfg += "  };\n\n"
    cfg += '  system.stateVersion = "26.05";\n'
    cfg += "}\n"
    return cfg, desktop, profile, boot_others


FLAKE = """\
{
  description = "Raze Linux system";

  # The Raze tree (modules, packages, pinned nixpkgs) the system is built from.
  # Update with: sudo rix update && sudo rix system switch
  inputs.raze.url = "path:./raze";

  outputs = { self, raze }: {
    nixosConfigurations.raze = raze.lib.mkSystem {
      modules = [ ./configuration.nix ./hardware-configuration.nix ];
    };
  };
}
"""


class Progress:
    """Turns nixos-install --log-format internal-json into a progress fraction."""

    def __init__(self):
        self.started = 0
        self.last_msg = None

    def handle(self, line):
        if not line.startswith("@nix "):
            return False
        try:
            ev = json.loads(line[5:])
        except ValueError:
            return True
        if ev.get("action") == "start" and ev.get("type") in (100, 101, 105, 108):
            self.started += 1
            text = ev.get("text") or ""
            if text:
                self.last_msg = text
        elif ev.get("action") == "msg" and ev.get("level", 5) <= 3:
            self.last_msg = ev.get("msg")
        return True

    @property
    def fraction(self):
        # Number of store operations is not known up front; approach 1 asymptotically.
        return 1.0 - 1.0 / (1.0 + self.started / 150.0)


def run():
    gs = libcalamares.globalstorage
    root = gs.value("rootMountPoint")
    if not root or not os.path.ismount(root):
        return ("Target not mounted", "The installation target {} is not mounted.".format(root))
    etc_nixos = os.path.join(root, "etc/nixos")
    log_path = os.path.join(root, "var/log/raze-install.log")
    os.makedirs(os.path.dirname(log_path), exist_ok=True)
    log = open(log_path, "w")

    def logline(msg):
        log.write(msg.rstrip("\n") + "\n")
        log.flush()
        libcalamares.utils.debug("raze: " + msg.rstrip("\n"))

    set_status("Detecting hardware", 0.02)
    try:
        out = subprocess.run(
            ["nixos-generate-config", "--root", root],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=True,
        ).stdout.decode("utf-8", "replace")
        logline(out)
    except subprocess.CalledProcessError as e:
        out = e.stdout.decode("utf-8", "replace") if e.stdout else ""
        logline(out)
        return ("Hardware detection failed", out)

    set_status("Looking for other operating systems", 0.04)
    others = os_prober()
    logline("os-prober: {}".format(others))

    set_status("Writing system configuration", 0.05)
    try:
        cfg, desktop, profile, boot_others = generate_configuration(gs, root, others)
    except ValueError as e:
        return ("Invalid installer input", str(e))
    with open(os.path.join(etc_nixos, "configuration.nix"), "w") as f:
        f.write(cfg)
    with open(os.path.join(etc_nixos, "flake.nix"), "w") as f:
        f.write(FLAKE)
    target_src = os.path.join(etc_nixos, "raze")
    if os.path.exists(target_src):
        shutil.rmtree(target_src)
    shutil.copytree(os.path.realpath(RAZE_SOURCE), target_src, symlinks=True)
    for dirpath, dirnames, filenames in os.walk(target_src):
        os.chmod(dirpath, 0o755)
        for fn in filenames:
            p = os.path.join(dirpath, fn)
            if not os.path.islink(p):
                os.chmod(p, 0o644)
    logline("desktop={} profile={} other systems in boot menu: {}".format(desktop, profile, boot_others))
    logline(cfg)

    # Lock the system flake before nixos-install. Otherwise nixos-install hashes
    # /etc/nixos, then writes flake.lock into it, and the hash no longer matches.
    set_status("Locking system configuration", 0.07)
    lock = subprocess.run(
        ["nix", "--extra-experimental-features", "nix-command flakes", "flake", "lock", etc_nixos],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    logline(lock.stdout.decode("utf-8", "replace"))
    if lock.returncode != 0:
        log.close()
        return ("Could not lock the system configuration", lock.stdout.decode("utf-8", "replace")[-4000:])

    set_status("Installing Raze Linux", 0.08)
    cmd = [
        "nixos-install",
        "--no-root-passwd",
        "--no-channel-copy",
        "--root", root,
        "--flake", etc_nixos + "#raze",
        "--log-format", "internal-json",
        # Nix refuses build dirs below world-writable parents (the target is under /tmp).
        "--option", "build-dir", "/nix/var/nix/builds",
    ]
    logline("$ " + " ".join(cmd))
    progress = Progress()
    tail = []
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    for raw in proc.stdout:
        line = raw.decode("utf-8", "replace")
        if not progress.handle(line):
            logline(line)
            tail.append(line)
            tail = tail[-60:]
        elif progress.last_msg:
            logline(progress.last_msg)
            progress.last_msg = None
        libcalamares.job.setprogress(0.08 + 0.9 * progress.fraction)
    code = proc.wait()
    logline("nixos-install exited with {}".format(code))
    log.close()
    if code != 0:
        return ("nixos-install failed", "".join(tail) + "\nFull log: /var/log/raze-install.log on the target disk.")

    set_status("Installation complete", 1.0)
    return None
