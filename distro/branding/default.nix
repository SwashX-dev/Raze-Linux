{ lib, stdenvNoCC, resvg, niri, version }:
stdenvNoCC.mkDerivation {
  pname = "raze-branding";
  inherit version;
  src = lib.fileset.toSource { root = ./.; fileset = lib.fileset.unions [ ./art ./desktop ./scripts ]; };
  nativeBuildInputs = [ resvg niri ];
  dontConfigure = true;
  buildPhase = ''
    runHook preBuild
    resvg --width 3840 --height 2160 art/wallpaper.svg wallpaper.png
    resvg --width 1920 --height 1080 art/wallpaper.svg wallpaper-1080.png
    for s in 64 128 256 512; do resvg --width $s --height $s art/logo.svg logo-$s.png; done
    # Reject a broken compositor config at build time, not at login.
    niri validate -c desktop/niri/config.kdl
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
    install -Dm755 -t $out/bin scripts/*
    runHook postInstall
  '';
  meta = {
    description = "Raze Linux artwork and desktop session defaults";
    license = lib.licenses.mit;
  };
}
