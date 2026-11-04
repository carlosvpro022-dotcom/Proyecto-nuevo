# Alojar el servidor en el mismo PC

Esta guía monta NEXUS REMOTE **sin alquilar ningún servidor**: el canal de
señalización corre en el mismo PC con Windows que se va a controlar.

**Coste: 0 €**, o unos 10-15 € al año si quiere un dominio propio en lugar de
uno gratuito.

---

## Antes de empezar: la comprobación que lo decide todo

Para que el iPhone encuentre su PC desde fuera de casa, su operador tiene que
darle una **IP pública de verdad**. Muchos operadores reparten una IP entre
varios clientes (se llama CGNAT) y entonces **no se puede abrir ningún puerto**
hacia su casa, haga lo que haga en el router.

Compruébelo así, en dos minutos:

1. Entre en la página de administración de su router (suele ser
   `192.168.1.1`) y busque la **IP de internet** o **IP WAN**.
2. Desde el mismo PC, abra <https://api.ipify.org> en el navegador.
3. Compare los dos números.

| Resultado | Qué significa |
| --- | --- |
| **Coinciden** | Tiene IP pública. Siga con esta guía. |
| **No coinciden**, y el router muestra algo que empieza por `100.64`–`100.127` o por `10.` | Es CGNAT. Llame a su operador y pida una IP pública: muchos la dan gratis si se les pide. Si se niegan, esta guía no le sirve y hay que alquilar un servidor. |

---

## Lo que vamos a montar

Tres piezas, todas en su PC:

```
Internet
   │
   ├── TCP 80, 443 ──→  caddy.exe ──→ nexus-signaling.exe
   │                    (cifrado)      (solo escucha en 127.0.0.1)
   │
   └── UDP 51820  ──────────────────→  la sesión cifrada (pantalla, teclado)
```

- **`nexus-signaling.exe`** es el canal de señalización. Escucha **solo en la
  propia máquina**: desde internet no se llega a él directamente.
- **`caddy.exe`** pone el cifrado TLS y consigue el certificado solo. Es lo
  único que mira a internet por TCP.
- El **puerto UDP** es por donde va la sesión: la pantalla, el teclado y los
  archivos, cifrados extremo a extremo.

No hace falta Docker, ni WSL, ni una máquina Linux. Tampoco hace falta
**Coturn**: el relay sirve para cuando los dos extremos no pueden verse, y aquí
su PC es directamente alcanzable. Lo que se pierde está explicado al final.

---

## 1. Un nombre para su casa

Su IP doméstica cambia sola cada cierto tiempo, así que no sirve apuntar a un
número. Hace falta un nombre que se actualice solo.

**Gratis:** regístrese en [DuckDNS](https://www.duckdns.org) o
[No-IP](https://www.noip.com) y cree un nombre, por ejemplo
`micasa.duckdns.org`. Instale su pequeño actualizador en el PC; se encarga de
que el nombre apunte siempre a su IP actual.

**Con dominio propio:** si prefiere algo como `remoto.suapellido.com`, compre
el dominio (unos 10-15 € al año) y use el DNS dinámico de su proveedor.

Compruébelo desde una consola:

```powershell
nslookup micasa.duckdns.org
```

Debe devolver la misma IP que vio en <https://api.ipify.org>.

---

## 2. Fijar la dirección local del PC

Si su PC cambia de dirección dentro de casa, el reenvío de puertos deja de
apuntar a él. En el router, busque **DHCP** → **reserva** o **IP estática** y
fije la dirección de su PC (algo como `192.168.1.50`).

Para saber cuál es ahora:

```powershell
ipconfig | findstr IPv4
```

---

## 3. Abrir los puertos en el router

Busque en su router **Port forwarding**, **Reenvío de puertos** o **Servidores
virtuales**, y añada tres reglas apuntando a la IP que acaba de fijar:

| Puerto externo | Protocolo | Puerto interno | Para qué |
| --- | --- | --- | --- |
| 80 | TCP | 80 | Conseguir y renovar el certificado |
| 443 | TCP | 443 | El canal de señalización, cifrado |
| 51820 | UDP | 51820 | La sesión: pantalla, teclado, archivos |

El 51820 es una elección libre; use el que quiera por encima de 1024, pero
anótelo porque luego hay que ponerlo en el panel.

> **Windows le preguntará** si permite que cada programa acepte conexiones la
> primera vez. Diga que sí para redes privadas y públicas.

---

## 4. Los dos programas

Cree una carpeta, por ejemplo `C:\NexusRemote\servidor`, y ponga dentro:

- **`nexus-signaling.exe`**, que sale de compilar este proyecto:
  ```bash
  cargo build --release -p nexus-signaling
  ```
  El ejecutable queda en `target/release/nexus-signaling.exe`.

- **`caddy.exe`**, que se descarga de <https://caddyserver.com/download>
  (elija Windows / amd64).

### El archivo `Caddyfile`

En la misma carpeta, cree un archivo llamado `Caddyfile` **sin extensión**:

```caddyfile
{
	email usted@sucorreo.com
	admin off
}

micasa.duckdns.org {
	encode zstd gzip

	header {
		Strict-Transport-Security "max-age=31536000; includeSubDomains"
		X-Content-Type-Options "nosniff"
		X-Frame-Options "DENY"
		Referrer-Policy "no-referrer"
		-Server
	}

	handle /v1/* {
		reverse_proxy 127.0.0.1:8443
	}
	handle /health {
		reverse_proxy 127.0.0.1:8443
	}
	handle /signaling* {
		rewrite * /v1/signaling
		reverse_proxy 127.0.0.1:8443
	}

	handle {
		respond "Not Found" 404
	}
}
```

Cambie `micasa.duckdns.org` por su nombre y el correo por el suyo.

### El archivo `arrancar.bat`

```bat
@echo off
cd /d "%~dp0"

rem El canal escucha SOLO en la propia maquina. Desde internet se llega a el
rem a traves de Caddy, que es quien pone el cifrado.
set NEXUS_BIND_ADDRESS=127.0.0.1:8443
set NEXUS_PUBLIC_URL=wss://micasa.duckdns.org/v1/signaling

rem Testigo que este PC debe presentar la primera vez. Generelo una vez con
rem una contrasena larga y aleatoria, y no lo reutilice para nada mas.
set NEXUS_HOST_ENROLLMENT_TOKEN=PEGUE_AQUI_UN_SECRETO_LARGO_Y_ALEATORIO

rem Sin relay TURN, a proposito: este equipo se alcanza directamente.
set NEXUS_ALLOW_NO_RELAY=true

rem Caddy va delante y es quien sabe de donde viene cada conexion.
set NEXUS_TRUST_PROXY_HEADER=true

set NEXUS_LOG=info

start "Caddy" caddy.exe run --config Caddyfile
start "Senalizacion" nexus-signaling.exe
```

Para generar el testigo, en PowerShell:

```powershell
[Convert]::ToBase64String((1..36 | ForEach-Object { Get-Random -Max 256 }))
```

Guárdelo donde guarde sus contraseñas. Lo necesitará una sola vez.

---

## 5. Arrancar y comprobar

Haga doble clic en `arrancar.bat`. Se abren dos ventanas; déjelas abiertas.

La primera vez, Caddy tarda unos segundos en conseguir el certificado. Después:

```powershell
curl https://micasa.duckdns.org/health
```

Debe contestar `{"status":"ok", ...}`. Si da un error de certificado, espere un
minuto y repita; si persiste, mire la ventana de Caddy.

**Desde fuera de casa** (con el móvil en datos, no en el WiFi), abra
`https://micasa.duckdns.org/v1/info` en el navegador. Si ve el JSON del
servicio, el reenvío de puertos funciona.

---

## 6. Configurar el host

En el panel de NEXUS REMOTE, pantalla **Configuración**:

| Campo | Valor |
| --- | --- |
| URL del servidor | `wss://micasa.duckdns.org/v1/signaling` |
| Puerto UDP | `51820` (el que abrió en el router) |
| Dirección pública | `micasa.duckdns.org` |

Los dos últimos son lo que hace que funcione desde fuera: con un puerto al azar
no habría nada que reenviar, y sin la dirección pública su PC anunciaría su
dirección privada, que desde internet no lleva a ninguna parte.

Pulse **Probar conexión** y pegue el testigo de alta. Debe decir *Conectado y
disponible*. Avisará de que no hay relay TURN: en este montaje es correcto.

Después, en el **Panel**, pulse **Empezar a atender**.

---

## 7. Que arranque solo con el PC

Pulse <kbd>Win</kbd>+<kbd>R</kbd>, escriba `shell:startup` y copie ahí un
acceso directo a `arrancar.bat`.

Para algo más serio —que arranque sin iniciar sesión y se reinicie si falla—
use el Programador de tareas: tarea nueva, disparador *Al iniciar el equipo*,
acción *Iniciar programa* → `arrancar.bat`, y marque *Ejecutar tanto si el
usuario inició sesión como si no*.

---

## Lo que este montaje no da

Conviene saberlo antes de depender de él.

**No hay relay.** Si la red desde la que se conecta el iPhone bloquea el tráfico
UDP saliente —ocurre en algunas redes de empresa y de hoteles—, la sesión no se
establecerá. Desde una red móvil normal o un WiFi doméstico funciona. Si llega a
molestarle, la solución es alquilar un servidor pequeño y poner ahí Coturn; la
guía está en [`despliegue.md`](despliegue.md).

**El servidor solo está encendido cuando el PC lo está.** Lo cual no importa:
si el PC está apagado, no hay nada que controlar.

**Hay un puerto de su casa abierto a internet.** Lo que queda expuesto es Caddy
y, detrás, el canal de señalización, que:

- solo escucha en `127.0.0.1`, de modo que únicamente Caddy llega a él;
- rechaza cualquier conexión que no demuestre tener una clave registrada;
- corta los mensajes grandes antes de almacenarlos;
- limita los intentos por dispositivo y por dirección de origen;
- no expone ninguna ruta de administración.

Aun así, un puerto abierto es un puerto abierto. Mantenga el PC actualizado y el
testigo de alta donde guarda sus contraseñas.

**Si su IP cambia**, el DNS dinámico lo arregla solo, pero una sesión que
estuviera abierta en ese momento se cae y hay que volver a conectar.

---

## Si algo no funciona

| Síntoma | Dónde mirar |
| --- | --- |
| `curl` al `/health` falla desde el propio PC | ¿Están las dos ventanas abiertas? Mire la de señalización. |
| Funciona en casa pero no desde fuera | Reenvío de puertos, o CGNAT (apartado inicial). |
| Error de certificado | El puerto 80 tiene que llegar hasta Caddy. Mire su ventana. |
| *Probar conexión* dice que rechazó la identidad | Falta el testigo de alta, o no coincide con el del `.bat`. |
| Conecta pero la sesión no llega a abrirse | El puerto UDP del router y el del panel tienen que ser el mismo. |
