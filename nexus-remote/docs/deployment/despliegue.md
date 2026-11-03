# Despliegue de la infraestructura

Esta guía describe cómo poner en marcha el servidor propio: señalización, STUN
y TURN. Sin él, los dispositivos solo se encontrarían dentro de la misma red
local.

> **Estado:** los archivos de despliegue están escritos y `docker compose
> config` los valida, pero **este despliegue todavía no se ha ejecutado**. El
> entorno donde se desarrolló no tiene demonio de Docker. Trate esta guía como
> el procedimiento previsto, no como un procedimiento verificado.

## 1. Lo que hace falta antes de empezar

| Requisito | Detalle |
| --- | --- |
| Servidor | 1 vCPU y 1 GB de RAM bastan para uso personal. El cuello de botella es el ancho de banda, no la CPU. |
| Sistema | Cualquier Linux con Docker y el plugin Compose. |
| IP pública | Fija, o con DNS dinámico. Detrás de CGNAT **no funciona**: el TURN necesita ser alcanzable. |
| Dominio | Un nombre con registro `A` (y `AAAA` si hay IPv6) apuntando al servidor. |
| Ancho de banda | Cuente unos 5 Mbps de subida y 5 de bajada por cada sesión que pase por el relay. |

El dominio es imprescindible: Let's Encrypt no emite certificados para
direcciones IP, y sin TLS la señalización iría en claro.

## 2. Puertos y cortafuegos

| Puerto | Protocolo | Servicio | Por qué |
| --- | --- | --- | --- |
| 80 | TCP | Caddy | Validación de Let's Encrypt. Solo eso. |
| 443 | TCP | Caddy | Señalización sobre WSS. |
| 3478 | UDP y TCP | Coturn | STUN y TURN sin cifrar. |
| 5349 | TCP | Coturn | TURN sobre TLS. Atraviesa redes que solo dejan salir por puertos de TLS. |
| 49160–49200 | UDP | Coturn | Retransmisión de medios. |

El rango UDP limita las sesiones simultáneas. Para uso personal sobra;
amplíelo si hace falta y ajuste `NEXUS_TURN_MIN_PORT` y `NEXUS_TURN_MAX_PORT`.

Con `ufw`:

```bash
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw allow 3478
sudo ufw allow 5349/tcp
sudo ufw allow 49160:49200/udp
sudo ufw enable
```

En AWS, GCP o Azure hay que abrirlos además en el grupo de seguridad, y
rellenar `NEXUS_EXTERNAL_IP` con la IP pública: esas máquinas están detrás de
NAT y Coturn anunciaría la dirección privada.

## 3. Configuración

```bash
git clone <este-repositorio>
cd nexus-remote/infrastructure/docker
cp .env.example .env
chmod 600 .env
```

Edite `.env`. El valor que **no debe inventarse** es el secreto de TURN:

```bash
openssl rand -base64 48
```

Péguelo en `NEXUS_TURN_SHARED_SECRET`. El servidor se niega a arrancar si tiene
menos de 32 caracteres, y el contenedor de Coturn también lo comprueba.

Todas las variables están documentadas una a una en `.env.example`.

## 4. Certificados

Caddy los obtiene y renueva solo, siempre que el dominio apunte a la máquina y
el puerto 80 esté abierto. No hay que hacer nada.

Coturn necesita esos mismos certificados para servir TURNS en el 5349. El
`compose.yaml` comparte el volumen de Caddy, pero **Coturn no recarga los
certificados al renovarse**: hay que reiniciarlo. Añada una tarea mensual:

```cron
0 4 1 * * cd /ruta/a/nexus-remote/infrastructure/docker && docker compose restart coturn
```

## 5. Arranque

```bash
docker compose up -d
docker compose ps
docker compose logs -f signaling
```

## 6. Comprobar que funciona

**Salud del servicio:**

```bash
curl https://nexus.midominio.com/health
# {"status":"ok","uptime_secs":42}

curl https://nexus.midominio.com/v1/info
# Debe mostrar "relay_configured": true
```

Fíjese en que `/v1/info` lista `/v1/signaling` en `pending_endpoints`: ese canal
llega en la etapa 3.

**Que el TURN responde de verdad.** Es la comprobación que más se olvida y la
que más problemas evita. Use la herramienta de prueba de WebRTC:

1. Abra <https://webrtc.github.io/samples/src/content/peerconnection/trickle-ice/>.
2. Añada `turns:nexus.midominio.com:5349?transport=tcp` con un usuario y
   credencial temporales.
3. Pulse *Gather candidates*. Debe aparecer al menos un candidato de tipo
   **relay**. Si solo aparecen `host` y `srflx`, el TURN no está sirviendo.

Para generar unas credenciales de prueba, con el mismo secreto del `.env`:

```bash
SECRET="el-valor-de-NEXUS_TURN_SHARED_SECRET"
USER="$(( $(date +%s) + 3600 )):prueba"
echo "usuario: $USER"
echo -n "$USER" | openssl dgst -binary -sha1 -hmac "$SECRET" | openssl base64
```

**Que el puerto está realmente abierto**, desde otra máquina:

```bash
nc -zv nexus.midominio.com 3478
nc -zv nexus.midominio.com 5349
```

## 7. Configurar el host de Windows

En el panel del host, pantalla **Configuración**, escriba:

```
wss://nexus.midominio.com/signaling
```

Debe empezar por `wss://`. El host rechaza cualquier otra cosa: sin TLS, la
señalización expondría los identificadores de los dispositivos y los candidatos
de red.

## 8. Mantenimiento

```bash
# Ver el consumo
docker stats

# Registros
docker compose logs --tail 200 signaling
docker compose logs --tail 200 coturn

# Actualizar tras cambios en el código
git pull && docker compose build signaling && docker compose up -d

# Copia de seguridad: lo único que hay que guardar es .env.
# Todo lo demás se reconstruye desde el repositorio.
```

## 9. Si algo no funciona

| Síntoma | Causa probable |
| --- | --- |
| Caddy no obtiene el certificado | El dominio no apunta a la máquina, o el puerto 80 está cerrado. Mire `docker compose logs caddy`. |
| `signaling` no arranca | Falta una variable obligatoria. El registro dice cuál y el proceso termina con código 1. |
| No aparecen candidatos `relay` | Rango UDP cerrado, `NEXUS_EXTERNAL_IP` sin rellenar detrás de NAT, o el secreto no coincide entre los dos servicios. |
| Funciona en la red local pero no desde fuera | Es el síntoma clásico de TURN mal configurado. Repita el paso 6. |
| TURNS falla pero TURN funciona | Coturn no ve los certificados, o no se reinició tras una renovación. |

## 10. Notas de seguridad del despliegue

- `signaling` **no publica ningún puerto**: solo lo alcanza Caddy por la red
  interna del Compose.
- El contenedor de `signaling` corre como usuario sin privilegios, con el
  sistema de archivos en solo lectura y `no-new-privileges`.
- Coturn tiene desactivada su consola de administración y deniega la
  retransmisión hacia direcciones privadas y de bucle: un TURN que puede hablar
  con `127.0.0.1` o con `10.0.0.0/8` se convierte en un salto hacia la red
  interna del servidor.
- El archivo `.env` contiene el secreto compartido. No lo suba al repositorio y
  déjelo en modo `600`.
