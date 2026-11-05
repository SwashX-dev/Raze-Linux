{ lib, stdenv, libglvnd, libgbm, version }:
stdenv.mkDerivation {
  pname = "raze-gpu-check";
  inherit version;
  src = ./raze-gpu-check.c;
  dontUnpack = true;
  buildInputs = [ libglvnd libgbm ];
  buildPhase = ''
    $CC -O2 -Wall -Wextra -Werror -o raze-gpu-check $src -lEGL -lGLESv2 -lgbm
  '';
  installPhase = "install -Dm755 raze-gpu-check $out/bin/raze-gpu-check";
  meta = {
    description = "Detect whether a hardware-accelerated OpenGL ES renderer is available";
    license = lib.licenses.mit;
    mainProgram = "raze-gpu-check";
  };
}
