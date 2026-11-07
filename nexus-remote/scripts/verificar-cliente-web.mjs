// Comprueba el cliente web contra un host de verdad, en un navegador de verdad.
//
// Lo que se ejecuta aqui: el servidor de senalizacion real, el bucle real del
// host y Chromium abriendo la pagina. La pagina genera su clave Ed25519, se
// autentica, pide sesion, firma la atadura, pide la pantalla y la muestra.
//
// Lo unico que no es real son los pixeles del PC: la captura solo funciona en
// Windows, asi que el host de pruebas usa un capturador sintetico.
//
// Uso: node scripts/verificar-cliente-web.mjs [--ver]
//   --ver  deja las capturas en target/capturas-cliente-web

import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { extname, join, resolve } from 'node:path';

// Playwright puede estar en el proyecto o instalado de forma global. Se busca
// en los mismos sitios que `verificar-panel.mjs`, por el mismo motivo: exigir
// una instalacion concreta es lo que hace fallar este guion en otra maquina.
function cargarChromium() {
  const require = createRequire(import.meta.url);
  for (const candidato of [
    'playwright',
    'playwright-core',
    '/opt/node22/lib/node_modules/playwright',
    '/opt/node-tools/node_modules/playwright',
  ]) {
    try {
      return require(candidato).chromium;
    } catch {
      // Se prueba el siguiente.
    }
  }
  throw new Error('no se encontro Playwright. Instalelo con: npm install -D playwright');
}

const chromium = cargarChromium();

const RAIZ = resolve(import.meta.dirname, '..');
const DIST = join(RAIZ, 'apps/web-client/dist');
const CAPTURAS = join(RAIZ, 'target/capturas-cliente-web');
const TOPE = 60_000;

const TIPOS = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.map': 'application/json',
};

/** Sirve la pagina construida en 127.0.0.1, que cuenta como contexto seguro. */
async function servirPagina() {
  const servidor = createServer(async (peticion, respuesta) => {
    const ruta = (peticion.url ?? '/').split('?')[0];
    const archivo = join(DIST, ruta === '/' ? 'index.html' : ruta);
    try {
      const cuerpo = await readFile(archivo);
      respuesta.writeHead(200, { 'content-type': TIPOS[extname(archivo)] ?? 'application/octet-stream' });
      respuesta.end(cuerpo);
    } catch {
      respuesta.writeHead(404).end('no esta');
    }
  });
  await new Promise((r) => servidor.listen(0, '127.0.0.1', r));
  return { servidor, base: `http://127.0.0.1:${servidor.address().port}` };
}

/** Arranca el host de pruebas y espera su linea de presentacion. */
function arrancarHost(dispositivoId, clavePublicaHex) {
  const proceso = spawn(
    'cargo',
    ['run', '-q', '-p', 'nexus-windows-host', '--example', 'host_de_pruebas'],
    {
      cwd: RAIZ,
      env: {
        ...process.env,
        NEXUS_WEB_CONTROLADOR_ID: dispositivoId,
        NEXUS_WEB_CONTROLADOR_CLAVE: clavePublicaHex,
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );

  const registro = [];
  proceso.stderr.on('data', (trozo) => registro.push(String(trozo)));

  const presentacion = new Promise((resolver, rechazar) => {
    let pendiente = '';
    const plazo = setTimeout(() => {
      rechazar(new Error(`el host no se presento en 30 s\n${registro.join('')}`));
    }, 30_000);
    proceso.stdout.on('data', (trozo) => {
      pendiente += String(trozo);
      const salto = pendiente.indexOf('\n');
      if (salto >= 0) {
        clearTimeout(plazo);
        resolver(JSON.parse(pendiente.slice(0, salto)));
      }
    });
    proceso.on('exit', (codigo) => {
      clearTimeout(plazo);
      rechazar(new Error(`el host termino con codigo ${codigo}\n${registro.join('')}`));
    });
  });

  return { proceso, presentacion, registro };
}

const fallos = [];
const comprobar = (condicion, texto) => {
  console.log(`  ${condicion ? 'ok  ' : 'MAL '} ${texto}`);
  if (!condicion) fallos.push(texto);
};

let host;
let paginaServida;
let browser;

try {
  paginaServida = await servirPagina();
  const argumentos = ['--no-sandbox'];
  if (process.env.NEXUS_SIN_MDNS === '1') {
    argumentos.push('--disable-features=WebRtcHideLocalIpsWithMdns');
  }
  browser = await chromium.launch({ args: argumentos });
  console.log(`  ..   ${browser.version()}`);
  const contexto = await browser.newContext({ viewport: { width: 1024, height: 700 } });
  const page = await contexto.newPage();
  const consola = [];
  page.on('console', (m) => consola.push(`${m.type()}: ${m.text()}`));
  page.on('pageerror', (e) => consola.push(`pageerror: ${e.message}`));

  // 1. Primera visita: la pagina crea su identidad y la ensena.
  await page.goto(paginaServida.base, { waitUntil: 'load' });
  await page.waitForFunction(() => globalThis.nexus?.identidad !== null, null, { timeout: 15_000 });
  const identidad = await page.evaluate(() => globalThis.nexus.identidad);
  comprobar(/^[0-9a-f-]{36}$/.test(identidad.dispositivoId), 'el navegador se da un identificador');
  comprobar(identidad.clavePublicaHex.length === 64, 'y una clave publica Ed25519 de 32 bytes');

  // 2. El host, emparejado con esa clave.
  host = arrancarHost(identidad.dispositivoId, identidad.clavePublicaHex);
  const presentacion = await host.presentacion;
  console.log(`  ..   host en ${presentacion.url}`);

  // 3. Segunda visita con los datos: la misma identidad tiene que volver de
  //    IndexedDB, o el host veria un dispositivo desconocido.
  const destino = `${paginaServida.base}/?servidor=${encodeURIComponent(presentacion.url)}&host=${presentacion.host_id}`;
  await page.goto(destino, { waitUntil: 'load' });
  await page.waitForFunction(() => globalThis.nexus?.identidad !== null, null, { timeout: 15_000 });
  const segunda = await page.evaluate(() => globalThis.nexus.identidad);
  comprobar(
    segunda.clavePublicaHex === identidad.clavePublicaHex,
    'la identidad sobrevive a recargar la pagina',
  );

  // 4. La pantalla.
  try {
    await page.waitForFunction(() => globalThis.nexus?.pantallaViva === true, null, { timeout: TOPE });
    comprobar(true, 'la pista de video llega al navegador');
  } catch {
    const pasos = await page.evaluate(() => globalThis.nexus?.pasos ?? []);
    comprobar(false, `la pantalla no llego. Pasos: ${JSON.stringify(pasos)}`);
  }

  if (!fallos.length) {
    // Que llegue la pista no es que se decodifique: eso lo dice el contador.
    await page.waitForFunction(async () => (await globalThis.nexus.fotogramas()) > 0, null, {
      timeout: TOPE,
    }).catch(() => {});
    const fotogramas = await page.evaluate(() => globalThis.nexus.fotogramas());
    comprobar(fotogramas > 0, `el navegador decodifica H.264 (${fotogramas} fotogramas)`);
    if (fotogramas === 0) {
      // Dos fallos que se parecen: que no lleguen bytes (red) y que lleguen y
      // no se decodifiquen (codec). Esto los separa.
      const diagnostico = await page.evaluate(() => globalThis.nexus.diagnostico());
      console.log(`  ..   diagnostico de la pista:\n${JSON.stringify(diagnostico, null, 2)}`);
    }

    const medidas = await page.evaluate(() => {
      const v = document.querySelector('video');
      return { ancho: v.videoWidth, alto: v.videoHeight, listo: v.readyState };
    });
    comprobar(
      medidas.ancho > 0 && medidas.alto > 0,
      `la imagen tiene tamano (${medidas.ancho}x${medidas.alto})`,
    );

    // Y que no sea un rectangulo negro: se dibuja el fotograma y se mide.
    // Solo tiene sentido si hay imagen: medir un lienzo de 0x0 es un error.
    const tinta = medidas.ancho === 0 ? 0 : await page.evaluate(() => {
      const v = document.querySelector('video');
      const lienzo = document.createElement('canvas');
      lienzo.width = Math.min(v.videoWidth, 160);
      lienzo.height = Math.min(v.videoHeight, 90);
      const ctx = lienzo.getContext('2d');
      ctx.drawImage(v, 0, 0, lienzo.width, lienzo.height);
      const datos = ctx.getImageData(0, 0, lienzo.width, lienzo.height).data;
      let suma = 0;
      for (let i = 0; i < datos.length; i += 4) suma += datos[i] + datos[i + 1] + datos[i + 2];
      return suma / (datos.length / 4) / 3;
    });
    if (medidas.ancho > 0) {
      comprobar(tinta > 8, `la imagen no es un rectangulo negro (brillo medio ${tinta.toFixed(1)})`);
    }
  }

  await mkdir(CAPTURAS, { recursive: true });
  await page.screenshot({ path: join(CAPTURAS, 'cliente-web.png') });
  const pasos = await page.evaluate(() => globalThis.nexus?.pasos ?? []);
  await writeFile(join(CAPTURAS, 'pasos.txt'), pasos.join('\n') + '\n');
  if (consola.length) {
    await writeFile(join(CAPTURAS, 'consola.txt'), consola.join('\n') + '\n');
  }
  console.log(`\n  capturas en ${CAPTURAS}`);
  if (fallos.length) {
    console.log(`\n  registro del host:\n${host.registro.join('')}`);
  }
} finally {
  await browser?.close();
  paginaServida?.servidor.close();
  host?.proceso.kill('SIGKILL');
}

if (fallos.length) {
  console.error(`\n${fallos.length} comprobacion(es) fallida(s).`);
  process.exit(1);
}
console.log('\nCliente web verificado contra un host real.');
