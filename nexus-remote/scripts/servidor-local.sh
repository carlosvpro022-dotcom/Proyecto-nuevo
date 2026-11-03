#!/usr/bin/env bash
# Arranca el servidor de señalización en local, con una configuración de
# pruebas. No sirve para producción: no hay TLS y el secreto es de juguete.
#
#   ./scripts/servidor-local.sh

set -euo pipefail
cd "$(dirname "$0")/.."

export NEXUS_BIND_ADDRESS="${NEXUS_BIND_ADDRESS:-127.0.0.1:8443}"
export NEXUS_PUBLIC_URL="${NEXUS_PUBLIC_URL:-wss://localhost:8443/signaling}"
export NEXUS_TURN_SHARED_SECRET="${NEXUS_TURN_SHARED_SECRET:-$(openssl rand -base64 48)}"
export NEXUS_TURN_URLS="${NEXUS_TURN_URLS:-turn:localhost:3478?transport=udp}"
export NEXUS_STUN_URLS="${NEXUS_STUN_URLS:-stun:localhost:3478}"
export NEXUS_LOG="${NEXUS_LOG:-debug}"

cat <<FIN

  Servidor de señalización en modo local
  --------------------------------------
  Escucha en   http://${NEXUS_BIND_ADDRESS}
  Salud        http://${NEXUS_BIND_ADDRESS}/health
  Información  http://${NEXUS_BIND_ADDRESS}/v1/info

  Sin TLS y con un secreto generado al vuelo: solo para desarrollo.

FIN

exec cargo run -p nexus-signaling
