# Gestor de WhatsApp (estilo WhatsApp Web)

Panel web para gestionar los mensajes de tu número de **WhatsApp Business** sobre la
**Cloud API oficial de Meta**. Interfaz tipo WhatsApp Web: lista de chats, conversación
en tiempo real, multimedia, contactos, etiquetas, respuestas automáticas y catálogo.

No usa librerías de ingeniería inversa ni vincula tu WhatsApp personal con un QR: habla
con la API oficial, así que no hay riesgo de que Meta bloquee el número.

---

## Qué hace

| Área | Incluye |
|---|---|
| **Chats** | Lista con búsqueda (también dentro del texto de los mensajes), filtros, fijar, archivar, silenciar, no leídos |
| **Mensajes** | Texto, respuestas citadas, reacciones con emoji, separadores por día, carga de histórico hacia atrás |
| **Estados** | Enviando / enviado / entregado / leído / fallido, con el motivo exacto del error de Meta |
| **Multimedia** | Imágenes, vídeo, audio, documentos y stickers: subida, previsualización y descarga |
| **Tiempo real** | WebSocket con reconexión automática; los mensajes entran sin recargar |
| **Contactos** | Nombre propio, notas internas, búsqueda, marca de bloqueado |
| **Etiquetas** | Creación con color, asignación a contactos, filtrado de chats por etiqueta |
| **Difusión** | Listas de destinatarios y envío masivo (sustituto de los grupos, ver limitaciones) |
| **Plantillas** | Listado desde Meta, relleno de variables `{{1}}` con previsualización, envío |
| **Catálogo** | Productos del catálogo de comercio y envío de uno o varios a un chat |
| **Automatización** | Respuestas por palabra clave, de bienvenida y fuera de horario, con antiflood |
| **Métricas** | Actividad de 7 días, ventanas abiertas, envíos fallidos, contactos por etiqueta |

### Limitaciones de la API oficial (no son del proyecto)

1. **No hay grupos.** La Cloud API no expone ningún endpoint de grupos de WhatsApp.
   Por eso existe la sección *Difusión*: envía el mismo mensaje a cada miembro como
   conversación individual.
2. **Ventana de 24 h.** Pasadas 24 h desde el último mensaje del contacto, Meta solo
   permite reanudar con una **plantilla aprobada**. La interfaz muestra el tiempo
   restante y ofrece el selector de plantillas cuando la ventana se cierra.
3. **No hay histórico previo.** Solo se guarda lo que llega por webhook desde que
   conectas la aplicación.
4. **Número dedicado.** El número que uses en la Cloud API no puede estar activo a la
   vez en la app normal de WhatsApp.

---

## Puesta en marcha

### 1. Requisitos en Meta

En [developers.facebook.com](https://developers.facebook.com/apps) crea una app de tipo
Business y añádele el producto **WhatsApp**. De ahí necesitas cuatro datos:

| Variable | Dónde está |
|---|---|
| `WHATSAPP_ACCESS_TOKEN` | WhatsApp → API Setup. Para producción genera un token permanente de *System User* en Business Settings; el de prueba caduca en 24 h |
| `WHATSAPP_PHONE_NUMBER_ID` | WhatsApp → API Setup (es un ID numérico, no el teléfono) |
| `WHATSAPP_BUSINESS_ACCOUNT_ID` | WhatsApp → API Setup (WABA ID) |
| `META_APP_SECRET` | Configuración de la app → Básica |

Permisos que debe tener el token: `whatsapp_business_messaging`,
`whatsapp_business_management` y, para el catálogo, `catalog_management`.

### 2. Instalación

```bash
npm install
cp .env.example .env     # y rellena los valores
```

Genera una clave para proteger el panel:

```bash
node -e "console.log(require('crypto').randomBytes(24).toString('hex'))"
```

Ponla en `APP_API_KEY` y, la primera vez que abras el panel, pégala en **Ajustes →
Clave de acceso**.

### 3. Arranque

```bash
npm run dev     # backend en :3001 y frontend en :5173
```

Para producción:

```bash
npm run build
npm start       # sirve API y frontend juntos en :3001
```

### 4. Conectar el webhook

Meta necesita alcanzar tu servidor por HTTPS desde internet. En local, abre un túnel:

```bash
npx cloudflared tunnel --url http://localhost:3001
# o: ngrok http 3001
```

En el panel de Meta, WhatsApp → Configuration → Webhook:

- **Callback URL**: `https://TU-DOMINIO/webhook`
- **Verify token**: el mismo valor que pusiste en `WEBHOOK_VERIFY_TOKEN`
- **Campos suscritos**: marca `messages`

Pulsa *Verify and save*. Si falla, revisa los logs del servidor: imprime el motivo.

---

## Arquitectura

```
server/                      Node 22 + Express + SQLite (better-sqlite3)
  src/whatsapp/client.ts     Cliente del Graph API: envío, media, plantillas, catálogo
  src/whatsapp/webhook.ts    Verificación HMAC, ingesta de mensajes y estados
  src/services/store.ts      Persistencia de conversaciones y mensajes
  src/services/sender.ts     Envío optimista: guarda, manda, refleja el resultado
  src/services/autoreply.ts  Motor de reglas con enfriamiento por contacto
  src/services/media.ts      Descarga y guardado seguro de adjuntos
  src/routes/                API REST del panel
  src/realtime/hub.ts        WebSocket hacia el navegador

web/                         React 18 + TypeScript + Vite + Tailwind 4
  src/lib/store.ts           Estado global (zustand) y cliente de tiempo real
  src/components/            Lista de chats, burbujas, redactor, selectores
  src/views/                 Contactos, etiquetas, difusión, catálogo, ajustes
```

### Decisiones que conviene conocer

- **Los webhooks se verifican por firma HMAC** (`X-Hub-Signature-256`) comparando los
  bytes crudos del cuerpo. Sin `META_APP_SECRET` se rechazan, salvo que actives
  `ALLOW_UNSIGNED_WEBHOOKS=true` (solo para pruebas locales).
- **Idempotencia**: Meta reintenta las entregas. Cada mensaje y cada cambio de estado
  se registra en `processed_events`, así que un reintento no duplica nada.
- **Los estados nunca retroceden**: si `read` llega antes que `delivered` por
  desorden de red, se conserva el más avanzado.
- **Los adjuntos se descargan en segundo plano**, para devolver el `200` a Meta
  dentro de su plazo. El mensaje aparece al instante con el adjunto en estado
  *descargando*.
- **El envío es optimista**: el mensaje se guarda y se muestra como *enviando* antes de
  llamar a Meta. Si falla, queda marcado como fallido con el motivo, igual que WhatsApp.
- **Las respuestas automáticas tienen enfriamiento** por regla y contacto (bienvenida:
  una sola vez; fuera de horario: una por hora; palabra clave: una cada 5 min) para no
  spamear ni entrar en bucles.

---

## Problemas frecuentes

| Síntoma | Causa habitual |
|---|---|
| El panel responde `401` | Falta la `APP_API_KEY` en Ajustes, o no coincide con la del servidor |
| Meta no verifica el webhook | `WEBHOOK_VERIFY_TOKEN` distinto al del panel, o la URL no es HTTPS pública |
| Llegan `401` en `/webhook` | `META_APP_SECRET` incorrecto: la firma no cuadra |
| `(#131047)` al enviar | Ventana de 24 h cerrada: usa una plantilla |
| Las plantillas no cargan | Falta `WHATSAPP_BUSINESS_ACCOUNT_ID` o el permiso `whatsapp_business_management` |
| El catálogo sale vacío | No hay catálogo conectado a la WABA, o falta `catalog_management` |
| Los adjuntos no se descargan | Token caducado: los media de Meta expiran a los 30 días |

## Aviso

Guarda `.env` fuera del control de versiones (ya está en `.gitignore`). El token de
acceso permite enviar mensajes en nombre de tu empresa: trátalo como una contraseña.
