#!/bin/sh
# Genera la configuracion de Coturn sustituyendo las variables de entorno y
# arranca el servidor.
#
# Se comprueba lo imprescindible antes de arrancar: un Coturn con el secreto
# vacio aceptaria cualquier credencial.
set -eu

: "${NEXUS_TURN_SHARED_SECRET:?falta NEXUS_TURN_SHARED_SECRET}"
: "${NEXUS_DOMAIN:?falta NEXUS_DOMAIN}"
: "${NEXUS_TURN_MIN_PORT:=49160}"
: "${NEXUS_TURN_MAX_PORT:=49200}"
: "${NEXUS_EXTERNAL_IP:=}"

if [ "${#NEXUS_TURN_SHARED_SECRET}" -lt 32 ]; then
  echo "error: NEXUS_TURN_SHARED_SECRET debe tener al menos 32 caracteres." >&2
  echo "       Genere uno con: openssl rand -base64 48" >&2
  exit 1
fi

TEMPLATE=/etc/coturn/turnserver.conf.template
OUTPUT=/tmp/turnserver.conf

# Sustitucion acotada a las variables conocidas, para no tocar nada mas del
# archivo por accidente.
sed \
  -e "s|\${NEXUS_TURN_SHARED_SECRET}|${NEXUS_TURN_SHARED_SECRET}|g" \
  -e "s|\${NEXUS_DOMAIN}|${NEXUS_DOMAIN}|g" \
  -e "s|\${NEXUS_TURN_MIN_PORT}|${NEXUS_TURN_MIN_PORT}|g" \
  -e "s|\${NEXUS_TURN_MAX_PORT}|${NEXUS_TURN_MAX_PORT}|g" \
  -e "s|\${NEXUS_EXTERNAL_IP}|${NEXUS_EXTERNAL_IP}|g" \
  "$TEMPLATE" > "$OUTPUT"

# Si no hay IP externa configurada se quita la linea, en lugar de dejarla vacia
# (Coturn la rechazaria).
if [ -z "${NEXUS_EXTERNAL_IP}" ]; then
  sed -i '/^external-ip=$/d' "$OUTPUT"
fi

echo "coturn: configuracion generada para ${NEXUS_DOMAIN}"
exec turnserver -c "$OUTPUT" --no-cli
