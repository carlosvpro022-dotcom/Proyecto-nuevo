#!/usr/bin/env bash
# Pasa todas las comprobaciones del proyecto.
#
# Es lo mismo que ejecuta la integración continua, para que un fallo se vea
# antes de subir nada.
#
#   ./scripts/verificar.sh          # todo
#   ./scripts/verificar.sh --rapido # sin el panel ni el compilado en cruce

set -euo pipefail
cd "$(dirname "$0")/.."

RAPIDO=0
[[ "${1:-}" == "--rapido" ]] && RAPIDO=1

paso() { printf '\n\033[1;36m▸ %s\033[0m\n' "$1"; }
fallo=0
intentar() {
  local nombre="$1"; shift
  if "$@"; then
    printf '  \033[32m✓ %s\033[0m\n' "$nombre"
  else
    printf '  \033[31m✗ %s\033[0m\n' "$nombre"
    fallo=1
  fi
}

paso "Formato de Rust"
intentar "cargo fmt" cargo fmt --all --check

paso "Lints de Rust (clippy pedantic, los avisos son errores)"
intentar "cargo clippy" cargo clippy --workspace --all-targets -- -D warnings

paso "Pruebas de Rust"
intentar "cargo test" cargo test --workspace

paso "Tipos del panel"
intentar "tsc" bash -c "cd apps/windows-host && npx --no-install tsc --noEmit"

if [[ $RAPIDO -eq 1 ]]; then
  echo
  [[ $fallo -eq 0 ]] && echo "Comprobaciones rápidas superadas." || echo "Hay comprobaciones fallidas."
  exit $fallo
fi

paso "Compilación del panel"
intentar "vite build" bash -c "cd apps/windows-host && npx --no-install vite build"

paso "Render del panel con datos reales"
if intentar "volcado de datos" bash -c \
    "cargo run -q -p nexus-windows-host --example volcado_ui > target/panel-datos.json"; then
  intentar "8 pantallas" node scripts/verificar-panel.mjs target/panel-datos.json
fi

paso "Comprobación en cruce del código de Windows"
if rustup target list --installed | grep -q x86_64-pc-windows-gnu; then
  intentar "cargo check (windows-gnu)" \
    cargo check --target x86_64-pc-windows-gnu \
      -p nexus-core -p nexus-security -p nexus-capture -p nexus-input \
      -p nexus-audio -p nexus-transport -p nexus-file-transfer
else
  printf '  \033[33m· omitido\033[0m (añada el objetivo con: rustup target add x86_64-pc-windows-gnu)\n'
fi

echo
if [[ $fallo -eq 0 ]]; then
  printf '\033[1;32mTodas las comprobaciones superadas.\033[0m\n'
else
  printf '\033[1;31mHay comprobaciones fallidas.\033[0m\n'
fi
exit $fallo
