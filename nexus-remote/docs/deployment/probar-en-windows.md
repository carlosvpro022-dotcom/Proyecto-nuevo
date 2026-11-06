# Probar en Windows: qué comprobar y en qué orden

Hay una cantidad importante de código escrito para Windows que **nadie ha
ejecutado en Windows**: el compilador verificó tipos, nombres de API y firmas
mediante compilación en cruce, pero no el comportamiento. Eso es lo que esta
sesión convierte en verificado.

El orden importa: cada paso depende del anterior. Si uno falla, los siguientes
no dicen nada útil.

**No hace falta servidor ni router para los pasos 1 a 4.** Son el PC solo.

**Y el paso 1 es casi todo.** Si solo va a hacer uno, haga ese.

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
cd Proyecto-nuevo\nexus-remote
```

---

## 1. El informe: un comando

```powershell
cargo run -p nexus-windows-host --example diagnostico
```

La primera compilación tarda bastante: está compilando WebRTC entero. Después
escribe un informe en la consola. **Cópielo entero y mándemelo**: con eso solo,
casi todo lo que había que comprobar queda comprobado.

Este comando **no toca nada**. No escribe en el disco, no abre el portapapeles,
no apaga nada y no se conecta a ningún servidor: solo mira y cuenta.

Va primero, y no la interfaz, por un motivo: el panel es una aplicación Tauri,
y si falla al compilar o al arrancar, la información que haría falta para
arreglarlo se queda dentro. Esto es un programa de consola normal.

### Qué debería decir

| Línea | Debería decir | Si dice otra cosa |
| --- | --- | --- |
| **Captura de pantalla** | `OK ... N pantalla(s)` | Mándeme el mensaje: la enumeración usa cuatro llamadas distintas y el mensaje dice cuál falló |
| **Ratón y teclado** | `OK ... backend windows-sendinput` | Mándeme el mensaje |
| **Portapapeles** | `OK ... responde` | Mándeme el mensaje |
| **Apagar y reiniciar** | `OK ... esta cuenta puede apagar` | Si dice que no tiene permiso, es una cuenta limitada: anótelo, no es un fallo del programa |
| **Cifrado de la clave** | `OK ... almacen windows-dpapi` | Si dice que quedaría sin cifrar, eso sí es un fallo |

Y debajo, el detalle de **cada pantalla**. Compruebe que cuadra con la realidad,
sobre todo:

- Si tiene **más de un monitor**, que estén todos y en la posición correcta.
- Si usa **escalado** (125 %, 150 %…), que lo diga bien. Esto importa de verdad:
  las coordenadas del ratón se calculan con ese número, y si está mal, el cursor
  irá a un sitio distinto del que usted toque en el iPad.

> **Lo que esto no comprueba:** que la captura entregue fotogramas de verdad y
> que `SendInput` mueva el ratón de verdad. Para eso hacen falta los dos
> extremos, y eso es la etapa 4.

---

## 2. El panel

```powershell
cd apps\windows-host
npm install
npm run tauri dev
```

**Qué debería pasar:** se abre una ventana con la barra lateral de NEXUS REMOTE
y la pantalla *Panel*. Las mismas comprobaciones del paso 1 aparecen en
**Escritorio remoto**, y en **Configuración**, al final, debería decir que la
clave privada está cifrada.

**Si falla:** el error de compilación o de enlazado, copiado entero. Que el
paso 1 haya funcionado y este no acota mucho dónde está el problema: sería de
la interfaz, no del motor.

---

## 3. Carpetas compartidas

En **Configuración → Carpetas compartidas**, añada una carpeta de su PC (por
ejemplo `C:\Users\<usted>\Intercambio`, créela antes) y marque que se puede
escribir. Guarde.

**Debería:** aparecer en la tabla, y en **Archivos** verse listada.

**Pruebe a añadir una mal a propósito**, para ver que lo rechaza:

- Una ruta que no existe → debería decir que no se puede leer
- `C:\` entero → debería decir que elija una carpeta concreta
- Sin nombre → debería pedir un nombre

---

## 4. Emparejar (sin servidor todavía)

En **Conexión remota**, genere un código. Debería aparecer un código de 8
caracteres y un QR, con su cuenta atrás de 5 minutos.

No hay nada que lo canjee todavía —el cliente de iPhone es la etapa 4— así que
esto solo comprueba que la pantalla y la generación funcionan.

---

## 5. Ya con servidor: la conexión

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

1. **El informe del paso 1, entero.** Es lo que más dice.
2. **El mensaje exacto** de lo que falle, copiado, no descrito.
3. **En qué paso** de esta lista.

Y de lo que funcione, basta con decir qué pasos pasaron. Con eso, el estado de
cada pieza deja de ser «compilado» y pasa a ser «probado», que es la diferencia
que este proyecto no mezcla.

---

## Lo que seguirá sin poder probarse

Aunque todo lo anterior salga bien:

- **Ver la pantalla desde otro dispositivo**, porque no hay todavía un cliente
  que la muestre. Lo que sí se puede probar en el PC es el primer tramo: que la
  captura enumera sus pantallas y entrega fotogramas (paso 1). Codificar y
  transmitir ya está probado aquí de punta a punta, pero con píxeles
  sintéticos.
- **El ratón y el teclado de verdad**, por lo mismo: hacen falta los dos
  extremos.
- **Apagar y reiniciar.** El paso 2 comprueba que la cuenta *podría*; probarlo
  de verdad apaga el PC, y eso lo decide usted.
- **Todo lo que necesita el iPhone**, que es la etapa 4 y necesita un Mac con
  Xcode.
