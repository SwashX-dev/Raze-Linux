{ lib
, rustPlatform
, pkg-config
, libinput
, libwacom
, libgudev
, seatd
, elogind
, eudev
, libgbm
, libglvnd
, libxkbcommon
, wayland
, pixman
, libdrm
, version
}:

let
  # Keep systemd libraries out of the compositor: libudev comes from eudev, and
  # libseat talks to elogind (the logind used on r6 systems) instead of systemd-logind.
  libinputEudev = libinput.override {
    udev = eudev;
    libwacom = libwacom.override {
      udev = eudev;
      # One test depends on systemd-udev sysfs attribute ordering.
      libgudev = (libgudev.override { udev = eudev; }).overrideAttrs { doCheck = false; };
    };
  };
  seatdElogind = (seatd.override { systemdSupport = false; }).overrideAttrs (old: {
    buildInputs = [ elogind ];
    mesonFlags = [ "-Dlibseat-logind=elogind" "-Dlibseat-builtin=enabled" "-Dserver=enabled" ];
  });
in
rustPlatform.buildRustPackage {
  pname = "rift";
  inherit version;

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./src ./default-config.toml ];
  };

  cargoLock = {
    lockFile = ./Cargo.lock;
    outputHashes = {
      "smithay-0.7.0" = "sha256-YD7P5iZAr+0XdeURhq6DkRkFws6Dpn0zSU5Y9qvsyZQ=";
    };
  };

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ libinputEudev seatdElogind eudev libgbm libglvnd libxkbcommon wayland pixman libdrm ];

  # libwayland-server and libEGL are loaded at runtime with dlopen.
  postFixup = ''
    patchelf --add-rpath ${lib.makeLibraryPath [ wayland libglvnd libxkbcommon ]} $out/bin/rift
  '';

  postInstall = ''
    install -Dm644 default-config.toml $out/share/rift/default-config.toml
    install -Dm644 /dev/stdin $out/share/wayland-sessions/rift.desktop <<DESKTOP
    [Desktop Entry]
    Name=Rift
    Comment=Scrollable-tiling Wayland compositor
    Exec=rift
    Type=Application
    DesktopNames=rift
    DESKTOP
  '';

  passthru = {
    providedSessions = [ "rift" ];
    inherit libinputEudev seatdElogind;
  };

  meta = {
    description = "Scrollable-tiling Wayland compositor for Raze Linux, built on Smithay";
    homepage = "https://github.com/SwashX-dev/Raze-Linux";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "rift";
  };
}
