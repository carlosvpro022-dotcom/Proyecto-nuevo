# Probar en Windows: qué comprobar y en qué orden

Hay una cantidad importante de código escrito para Windows que **nadie ha
ejecutado en Windows**: el compilador verificó tipos, nombres de API y firmas
mediante compilación en cruce, pero no el comportamiento. Eso es lo que esta
sesión convierte en verificado.

El orden importa: cada paso depende del anterior. Si uno falla, los siguientes
no dicen nada útil.

**No hace falta servidor ni router para los pasos 1 a 5.** Son el PC solo.

---

## Antes de empezar

Necesita en el PC con Windows:

- **Rust** (`rustup`), desde <https://rustup.rs>
- **Node.js 20 o posterior**, desde <https://nodejs.org>
- **Microsoft Visual Studio Build Tools** con «Desarrollo para el escritorio
  con C++». Tauri lo necesita para enlazar.

Luego:

```powershell
git clone https://github.com/carlosvpro022-dotcom/Proyecto-nuevo
cd Proyecto-nuevo\nexus-remote\apps\windows-host
npm install
npm run tauri dev
```

La primera compilación tarda bastante: está compilando WebRTC entero.

---

## 1. Que arranque y se vea el panel

**Qué debería pasar:** se abre una ventana con la barra lateral de NEXUS REMOTE
y la pantalla *Panel*.

**Si falla:** el error de compilación o de enlazado. Cópielo entero.

---

## 2. Escritorio remoto → las cuatro comprobaciones

Vaya a **Escritorio remoto**. Hay una tarjeta «Motor de escritorio remoto» con
cuatro filas. Esto es lo más valioso de toda la sesión, porque comprueba de una
vez casi todo el código que nunca se ha ejecutado.

| Fila | Debería decir | Si dice otra cosa |
| --- | --- | --- |
| **Captura de pantalla** | «N pantalla(s) detectada(s)» | Anote el mensaje: la enumeración de monitores usa cuatro llamadas distintas y el mensaje dice cuál falló |
| **Control de ratón y teclado** | «backend windows-sendinput listo» | Anote el mensaje |
| **Portapapeles** | «el portapapeles responde» (con o sin texto dentro) | Anote el mensaje |
| **Apagar y reiniciar** | «esta cuenta puede apagar el equipo» | Si dice que no tiene permiso, es una cuenta limitada: anótelo, no es un fallo del programa |

Vaya también a **Mis dispositivos → (ninguno todavía)** y a **Configuración**.
En Configuración, al final, la tarjeta «Identidad de este host» debería decir
que **la clave privada está cifrada**. Si dice que está sin cifrar, eso es un
fallo y hay que verlo.

> **Lo que esto no comprueba:** que la captura entregue fotogramas de verdad y
> que `SendInput` mueva el ratón de verdad. Para eso hace falta una sesión con
> un dispositivo al otro lado, que es la etapa 4.

---

## 3. Las pantallas que detectó

Siga en **Escritorio remoto**, más abajo. Debería listar sus monitores con
resolución, posición, escala y refresco.

**Compruebe que cuadra con la realidad**, sobre todo:

- Si tiene **más de un monitor**, que estén todos y que la posición relativa
  sea la correcta.
- Si usa **escalado** (125 %, 150 %…), que lo diga bien. Esto importa de verdad:
  las coordenadas del ratón se calculan con ese número, y si está mal el cursor
  irá a un sitio distinto del que usted toque en el iPad.

---

## 4. Carpetas compartidas

En **Configuración → Carpetas compartidas**, añada una carpeta de su PC (por
ejemplo `C:\Users\<usted>\Intercambio`, créela antes) y marque que se puede
escribir. Guarde.

**Debería:** aparecer en la tabla, y en **Archivos** verse listada.

**Pruebe a añadir una mal a propósito**, para ver que lo rechaza:

- Una ruta que no existe → debería decir que no se puede leer
- `C:\` entero → debería decir que elija una carpeta concreta
- Sin nombre → debería pedir un nombre

---

## 5. Emparejar (sin servidor todavía)

En **Conexión remota**, genere un código. Debería aparecer un código de 8
caracteres y un QR, con su cuenta atrás de 5 minutos.

No hay nada que lo canjee todavía —el cliente de iPhone es la etapa 4— así que
esto solo comprueba que la pantalla y la generación funcionan.

---

## 6. Ya con servidor: la conexión

Esto necesita el servidor montado. La guía es
[`en-el-mismo-pc.md`](en-el-mismo-pc.md), y lo primero de todo es la
comprobación de CGNAT, que decide si el montaje le sirve.

Con el servidor en marcha, en **Configuración → Comprobar la conexión**, meta
el testigo de alta y pulse *Probar conexión*.

**Debería:** decir que conectó, cuántos servidores de red recibió, y si hay
retransmisión (TURN).

Sin TURN dirá que no, y es correcto: el montaje en su propio PC no lleva relay.
Lo que **sí** tiene que decir que sí es «alcanzable desde otra red».

---

## Qué reportar

De lo que falle, lo útil es:

1. **El mensaje exacto**, copiado, no descrito.
2. **En qué paso** de esta lista.
3. Si es la detección de pantallas: **cuántos monitores tiene y con qué escalado**.

Y de lo que funcione, basta con decir qué pasos pasaron. Con eso, el estado de
cada pieza deja de ser «compilado» y pasa a ser «probado», que es la diferencia
que este proyecto no mezcla.

---

## Lo que seguirá sin poder probarse

Aunque todo lo anterior salga bien:

- **La transmisión de pantalla**, porque todavía no hay codificador ni nada al
  otro lado que la reciba.
- **El ratón y el teclado de verdad**, por lo mismo: hacen falta los dos
  extremos.
- **Apagar y reiniciar.** El paso 2 comprueba que la cuenta *podría*; probarlo
  de verdad apaga el PC, y eso lo decide usted.
- **Todo lo que necesita el iPhone**, que es la etapa 4 y necesita un Mac con
  Xcode.
