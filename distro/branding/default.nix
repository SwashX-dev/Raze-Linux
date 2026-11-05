{ lib, stdenvNoCC, resvg, niri, foot, fuzzel, sway-unwrapped, version }:
stdenvNoCC.mkDerivation {
  pname = "raze-branding";
  inherit version;
  src = lib.fileset.toSource { root = ./.; fileset = lib.fileset.unions [ ./art ./desktop ./scripts ./fastfetch/raze.txt ./fastfetch/config.jsonc ]; };
  nativeBuildInputs = [ resvg niri foot fuzzel sway-unwrapped ];
  dontConfigure = true;
  buildPhase = ''
    runHook preBuild
    resvg --width 3840 --height 2160 art/wallpaper.svg wallpaper.png
    resvg --width 1920 --height 1080 art/wallpaper.svg wallpaper-1080.png
    for s in 64 128 256 512; do resvg --width $s --height $s art/logo.svg logo-$s.png; done
    # Reject broken session configs at build time, not at login.
    niri validate -c desktop/niri/config.kdl
    export XDG_RUNTIME_DIR=$TMPDIR HOME=$TMPDIR
    foot --config=desktop/foot.ini --check-config
    fuzzel --config=desktop/fuzzel.ini --check-config
    sed "s|@WALLPAPER@|/dev/null|" desktop/sway/raze.conf > sway-check.conf
    WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway --validate --config sway-check.conf
    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall
    install -Dm644 wallpaper.png $out/share/backgrounds/raze/wallpaper.png
    install -Dm644 wallpaper-1080.png $out/share/backgrounds/raze/wallpaper-1080.png
    install -Dm644 art/wallpaper.svg $out/share/backgrounds/raze/wallpaper.svg
    install -Dm644 art/logo.svg $out/share/pixmaps/raze-logo.svg
    install -Dm644 art/logo.svg $out/share/icons/hicolor/scalable/apps/raze-logo.svg
    for s in 64 128 256 512; do
      install -Dm644 logo-$s.png $out/share/icons/hicolor/''${s}x''${s}/apps/raze-logo.png
    done
    install -Dm644 logo-256.png $out/share/pixmaps/raze-logo.png
    mkdir -p $out/share/raze
    cp -r desktop $out/share/raze/desktop
    # Absolute wallpaper path: share/backgrounds is not linked into the system profile.
    wp=$out/share/backgrounds/raze/wallpaper.png
    substituteInPlace $out/share/raze/desktop/niri/config.kdl \
      --replace-fail /run/current-system/sw/share/backgrounds/raze/wallpaper.png $wp
    substituteInPlace $out/share/raze/desktop/sway/raze.conf --replace-fail @WALLPAPER@ $wp
    install -Dm755 -t $out/bin scripts/*
    # fastfetch: Raze logo and system-wide config (installed as /etc/xdg/fastfetch).
    install -Dm644 fastfetch/raze.txt $out/share/raze/fastfetch/raze.txt
    install -Dm644 fastfetch/config.jsonc $out/share/raze/fastfetch/config.jsonc
    substituteInPlace $out/share/raze/fastfetch/config.jsonc --replace-fail @logo@ $out/share/raze/fastfetch/raze.txt
    runHook postInstall
  '';
  meta = {
    description = "Raze Linux artwork and desktop session defaults";
    license = lib.licenses.mit;
  };
}
