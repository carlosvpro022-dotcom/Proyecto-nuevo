#!/usr/bin/env bash
# Comprueba e instala lo necesario para trabajar en NEXUS REMOTE.
#
# No instala nada sin decirlo: primero informa de lo que falta y luego, si se
# le pasa --instalar, lo instala.
#
#   ./scripts/preparar-entorno.sh            # solo diagnostica
#   ./scripts/preparar-entorno.sh --instalar # instala lo que falte (necesita sudo)

set -euo pipefail

INSTALAR=0
[[ "${1:-}" == "--instalar" ]] && INSTALAR=1

falta=()

info()  { printf '  \033[36m%-22s\033[0m %s\n' "$1" "$2"; }
ok()    { printf '  \033[32m✓\033[0m %-20s %s\n' "$1" "$2"; }
no()    { printf '  \033[31m✗\033[0m %-20s %s\n' "$1" "$2"; falta+=("$1"); }

echo
echo "NEXUS REMOTE · diagnóstico del entorno"
echo "======================================"
echo
echo "Sistema"
info "so" "$(uname -s) $(uname -r)"
info "arquitectura" "$(uname -m)"

echo
echo "Cadena de Rust"
if command -v cargo >/dev/null 2>&1; then
  ok "cargo" "$(cargo --version)"
  ok "rustc" "$(rustc --version)"
else
  no "rust" "instálelo desde https://rustup.rs"
fi

echo
echo "Cadena de Node"
if command -v node >/dev/null 2>&1; then
  ok "node" "$(node --version)"
  ok "npm" "$(npm --version)"
else
  no "node" "hace falta Node 20 o superior"
fi

echo
echo "Herramientas de compilación"
for t in gcc pkg-config git; do
  if command -v "$t" >/dev/null 2>&1; then
    ok "$t" "presente"
  else
    no "$t" "ausente"
  fi
done

echo
echo "Dependencias de Tauri"
case "$(uname -s)" in
  Linux)
    for lib in webkit2gtk-4.1 gtk+-3.0 libsoup-3.0; do
      if pkg-config --exists "$lib" 2>/dev/null; then
        ok "$lib" "$(pkg-config --modversion "$lib")"
      else
        no "$lib" "ausente"
      fi
    done
    ;;
  Darwin)
    info "macos" "Tauri usa WKWebView del sistema; no hace falta nada más"
    ;;
  *)
    info "windows" "WebView2 viene con Windows 10 2004 y posteriores"
    ;;
esac

echo
echo "Opcional"
for t in docker; do
  if command -v "$t" >/dev/null 2>&1; then
    ok "$t" "$("$t" --version 2>/dev/null | head -1)"
  else
    info "$t" "ausente (solo hace falta para desplegar el servidor)"
  fi
done

if command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  ok "mingw-w64" "permite comprobar el código de Windows en cruce"
else
  info "mingw-w64" "ausente (opcional: 'cargo check --target x86_64-pc-windows-gnu')"
fi

echo
if [[ ${#falta[@]} -eq 0 ]]; then
  echo "Todo lo necesario está presente."
  exit 0
fi

echo "Falta: ${falta[*]}"
echo

if [[ $INSTALAR -eq 0 ]]; then
  echo "Ejecute con --instalar para instalarlo, o hágalo a mano."
  exit 1
fi

if [[ "$(uname -s)" != "Linux" ]] || ! command -v apt-get >/dev/null 2>&1; then
  echo "La instalación automática solo está preparada para Debian y Ubuntu."
  exit 1
fi

echo "Instalando dependencias del sistema…"
sudo apt-get update
sudo apt-get install -y \
  build-essential pkg-config file curl wget \
  libwebkit2gtk-4.1-dev libssl-dev librsvg2-dev libxdo-dev \
  libayatana-appindicator3-dev libasound2-dev

if ! command -v cargo >/dev/null 2>&1; then
  echo "Instalando Rust…"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi

echo
echo "Listo. Vuelva a ejecutar el script sin --instalar para comprobarlo."
