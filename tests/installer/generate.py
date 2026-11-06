#!/usr/bin/env python3
"""Write the configuration.nix the Raze installer module (installer/calamares/
modules/raze/main.py) generates for several installation scenarios, so a flake check
can evaluate them. usage: generate.py MODULE_DIR OUT_DIR"""
import os
import sys
import tempfile
import types

module_dir, out = sys.argv[1], sys.argv[2]
sys.modules["libcalamares"] = types.ModuleType("libcalamares")
sys.path.insert(0, module_dir)
import main  # noqa: E402


class GS:
    def __init__(self, d):
        self.d = d

    def value(self, k):
        return self.d.get(k)


def esp(*loaders):
    root = tempfile.mkdtemp()
    for path in loaders:
        p = os.path.join(root, "boot/efi", path.strip("/"))
        os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "w").close()
    return root


user = {"username": "alice", "fullname": "Alice", "hostname": "raze"}
windows = [("/dev/vda1", "/efi/Microsoft/Boot/bootmgfw.efi", "Windows Boot Manager", "efi")]
scenarios = {
    "efi-erase": (GS({**user, "firmwareType": "efi"}), esp(), []),
    "efi-windows": (GS({**user, "firmwareType": "efi"}),
                    esp("EFI/Microsoft/Boot/bootmgfw.efi", "EFI/Microsoft/Boot/BCD",
                        "EFI/ubuntu/shimx64.efi", "EFI/BOOT/BOOTX64.EFI"), windows),
    "bios-windows": (GS({**user, "firmwareType": "bios", "bootLoader": {"installPath": "/dev/vda"}}),
                     esp(), [("/dev/vda1", None, "Windows 10", "chain")]),
    "gnome": (GS({**user, "firmwareType": "efi", "packagechooser_desktop": "gnome"}), esp(), []),
    "xfce": (GS({**user, "firmwareType": "efi", "packagechooser_desktop": "xfce"}), esp(), []),
}
os.makedirs(out, exist_ok=True)
for name, (gs, root, others) in scenarios.items():
    cfg, _desktop, _profile, _boot = main.generate_configuration(gs, root, others)
    with open(os.path.join(out, name + ".nix"), "w") as f:
        f.write(cfg)
