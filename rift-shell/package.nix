{ lib
, rustPlatform
, makeWrapper
, python3
, material-symbols
, quickshell
, grim
, slurp
, wl-clipboard
, cliphist
, libnotify
, brightnessctl
, wireplumber
, wf-recorder
, version
}:

rustPlatform.buildRustPackage {
  pname = "rift-shell";
  inherit version;

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./src ./shell ];
  };
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [ makeWrapper (python3.withPackages (p: [ p.fonttools p.brotli ])) ];

  postInstall = ''
    mkdir -p $out/share/rift-shell
    cp -r shell $out/share/rift-shell/shell

    # Only the Material Symbols glyphs Rift Shell uses, at one weight and optical size
    # (FILL stays variable), instead of the 15 MiB full font.
    # Icon names used by the shell: every quoted lower_snake_case string in the QML.
    grep -ohE '"[a-z][a-z0-9_]+"' shell/*.qml | tr -d '"' | sort -u > icons.txt
    src="$(echo ${material-symbols}/share/fonts/*/MaterialSymbolsRounded*.ttf)"
    fonttools varLib.instancer "$src" GRAD=0 opsz=24 wght=400 -o pinned.ttf
    pyftsubset pinned.ttf \
      --glyphs="$(paste -sd, icons.txt)" --text="abcdefghijklmnopqrstuvwxyz0123456789_" \
      --layout-features=liga,rlig,calt --ignore-missing-glyphs --no-hinting \
      --output-file=RiftShellSymbols.ttf
    install -Dm644 RiftShellSymbols.ttf $out/share/fonts/truetype/RiftShellSymbols.ttf
  '';

  postFixup = ''
    wrapProgram $out/bin/rift-shell \
      --prefix PATH : ${lib.makeBinPath [ quickshell grim slurp wl-clipboard cliphist libnotify brightnessctl wireplumber wf-recorder ]}
  '';

  meta = {
    description = "Rift Shell: panel, launcher, control centre, notifications and lock screen for Rift";
    homepage = "https://github.com/SwashX-dev/Raze-Linux";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "rift-shell";
  };
}
