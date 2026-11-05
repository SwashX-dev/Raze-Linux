# Raze Calamares extensions: settings, module configuration, the `raze` job module and
# branding. `calamares-raze` (passthru) is Calamares wrapped to use them.
{ lib, stdenvNoCC, calamares, glibcLocales, resvg, raze-branding, version }:
let
  ext = stdenvNoCC.mkDerivation {
    pname = "raze-calamares-extensions";
    inherit version;
    src = lib.fileset.toSource { root = ./.; fileset = lib.fileset.unions [ ./etc ./modules ./branding ]; };
    nativeBuildInputs = [ resvg ];
    dontConfigure = true;
    dontBuild = true;
    installPhase = ''
      runHook preInstall
      mkdir -p $out/etc/calamares $out/lib/calamares/modules $out/share/calamares/branding
      cp -r etc/. $out/etc/calamares/
      cp -r modules/. $out/lib/calamares/modules/
      cp -r branding/. $out/share/calamares/branding/
      substituteInPlace $out/etc/calamares/settings.conf --subst-var out
      substituteInPlace $out/etc/calamares/modules/locale.conf --replace-fail @locales@ ${glibcLocales}
      substituteInPlace $out/share/calamares/branding/raze/branding.desc --replace-fail @version@ ${version}
      b=$out/share/calamares/branding/raze
      cp ${raze-branding}/share/pixmaps/raze-logo.svg $b/logo.svg
      cp ${raze-branding}/share/icons/hicolor/256x256/apps/raze-logo.png $b/logo-256.png
      mkdir -p $b/images
      cp ${raze-branding}/share/backgrounds/raze/wallpaper-1080.png $b/images/raze.png
      runHook postInstall
    '';
    passthru.calamares = calamares.override {
      extraWrapperArgs = [
        "--prefix XDG_DATA_DIRS : ${ext}/share"
        "--prefix XDG_CONFIG_DIRS : ${ext}/etc"
        "--add-flag --xdg-config"
      ];
    };
    meta = {
      description = "Raze Linux installer configuration for Calamares";
      license = lib.licenses.mit;
    };
  };
in
ext
