# Raze Calamares extensions: settings, module configuration, the `raze` job module and
# branding. `calamares-raze` (passthru) is Calamares wrapped to use them.
{ lib, stdenvNoCC, runCommand, makeWrapper, calamares, glibcLocales, raze-branding, version }:
let
  ext = stdenvNoCC.mkDerivation {
    pname = "raze-calamares-extensions";
    inherit version;
    src = lib.fileset.toSource { root = ./.; fileset = lib.fileset.unions [ ./etc ./modules ./branding ]; };
    dontConfigure = true;
    dontBuild = true;
    installPhase = ''
      runHook preInstall
      mkdir -p $out/etc/calamares $out/lib/calamares/modules $out/share/calamares/branding
      cp -r etc/. $out/etc/calamares/
      cp -r modules/. $out/lib/calamares/modules/
      cp -r branding/. $out/share/calamares/branding/
      substituteInPlace $out/etc/calamares/settings.conf --subst-var out
      # Only the list of supported locales is needed, not the 220 MiB locale archive.
      install -Dm644 ${glibcLocales}/share/i18n/SUPPORTED $out/share/raze/SUPPORTED
      substituteInPlace $out/etc/calamares/modules/locale.conf --replace-fail @locales@/share/i18n/SUPPORTED $out/share/raze/SUPPORTED
      substituteInPlace $out/share/calamares/branding/raze/branding.desc --replace-fail @version@ ${version}
      b=$out/share/calamares/branding/raze
      cp ${raze-branding}/share/pixmaps/raze-logo.svg $b/logo.svg
      cp ${raze-branding}/share/icons/hicolor/256x256/apps/raze-logo.png $b/logo-256.png
      mkdir -p $b/images
      cp ${raze-branding}/share/backgrounds/raze/wallpaper-1080.png $b/images/raze.png
      runHook postInstall
    '';
    # Wrap the binary-cached Calamares instead of overriding its build (an override
    # recompiled Calamares whenever these files changed).
    passthru.calamares = runCommand "calamares-raze-${version}" { nativeBuildInputs = [ makeWrapper ]; } ''
      makeWrapper ${calamares}/bin/calamares $out/bin/calamares \
        --prefix XDG_DATA_DIRS : ${ext}/share \
        --prefix XDG_CONFIG_DIRS : ${ext}/etc \
        --add-flags --xdg-config
      ln -s ${calamares}/share $out/share
    '';
    meta = {
      description = "Raze Linux installer configuration for Calamares";
      license = lib.licenses.mit;
    };
  };
in
ext
