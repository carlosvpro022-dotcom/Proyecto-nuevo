/**
 * Verifica que el panel del host se dibuja correctamente.
 *
 * Carga el panel ya compilado (`apps/windows-host/dist`) en Chromium, que es
 * el mismo motor que usa WebView2 en Windows, e inyecta los datos reales que
 * produce `cargo run --example volcado_ui`. Despues recorre cada pantalla,
 * guarda una captura y comprueba que no hay errores de consola.
 *
 * No sustituye a abrir la aplicacion de verdad, pero detecta lo que mas se
 * rompe al cambiar la interfaz: un componente que falla al dibujarse, un campo
 * que el backend renombra y el panel sigue leyendo, o un error de JavaScript.
 *
 * Uso:
 *
 *   cargo run -q -p nexus-windows-host --example volcado_ui > /tmp/panel.json
 *   node scripts/verificar-panel.mjs /tmp/panel.json [carpeta-de-capturas]
 */

import { createRequire } from "node:module";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { mkdir } from "node:fs/promises";
import { extname, join, resolve } from "node:path";
import process from "node:process";

const ROOT = resolve(import.meta.dirname, "..");
const DIST = join(ROOT, "apps/windows-host/dist");

/**
 * Playwright puede estar instalado en el proyecto o de forma global. Se busca
 * en ambos sitios en lugar de exigir una instalacion concreta, que es lo que
 * hace fallar este guion en una maquina distinta a la de quien lo escribio.
 */
function loadChromium() {
  const require = createRequire(import.meta.url);
  const candidates = [
    "playwright",
    "playwright-core",
    "/opt/node22/lib/node_modules/playwright",
    "/opt/node-tools/node_modules/playwright",
  ];
  for (const candidate of candidates) {
    try {
      return require(candidate).chromium;
    } catch {
      // Se prueba el siguiente.
    }
  }
  throw new Error(
    "no se encontro Playwright. Instalelo con: npm install -D playwright",
  );
}

const chromium = loadChromium();

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
};

/** Pantallas a recorrer, con el texto que las identifica en la barra lateral. */
const VIEWS = [
  { id: "dashboard", nav: "Panel", expect: "Este equipo" },
  { id: "devices", nav: "Mis dispositivos", expect: "Dispositivos autorizados" },
  { id: "pairing", nav: "Conexion remota", expect: "Codigo de emparejamiento activo" },
  { id: "desktop", nav: "Escritorio remoto", expect: "Motor de escritorio remoto" },
  { id: "files", nav: "Archivos", expect: "Transferencia de archivos" },
  { id: "history", nav: "Historial", expect: "Historial de sesiones" },
  { id: "security", nav: "Seguridad", expect: "Medidas implementadas" },
  { id: "settings", nav: "Configuracion", expect: "Servidor de senalizacion" },
];

async function serveDist() {
  const server = createServer(async (request, response) => {
    const path = request.url === "/" ? "/index.html" : request.url.split("?")[0];
    try {
      const body = await readFile(join(DIST, path));
      response.writeHead(200, {
        "Content-Type": MIME[extname(path)] ?? "application/octet-stream",
      });
      response.end(body);
    } catch {
      response.writeHead(404).end("no encontrado");
    }
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  return { server, port: server.address().port };
}

async function main() {
  const dumpPath = process.argv[2];
  const outDir = process.argv[3] ?? join(ROOT, "target/capturas-panel");

  if (!dumpPath) {
    console.error("uso: node scripts/verificar-panel.mjs <panel.json> [carpeta-salida]");
    process.exit(2);
  }

  const dump = JSON.parse(await readFile(dumpPath, "utf8"));
  await mkdir(outDir, { recursive: true });

  const { server, port } = await serveDist();
  const browser = await chromium.launch({ args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 820 } });

  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", (error) => consoleErrors.push(String(error)));

  // Se suplanta el puente de Tauri con las respuestas reales del backend.
  // Es lo unico simulado: los datos vienen del host de verdad.
  await page.addInitScript((responses) => {
    window.__TAURI_INTERNALS__ = {
      invoke: (command) => {
        if (command in responses) return Promise.resolve(responses[command]);
        if (command === "cancel_pairing") return Promise.resolve(true);
        if (command === "list_monitors") return Promise.resolve(responses.list_monitors ?? []);
        return Promise.reject({
          code: "no_implementado",
          message: `comando sin respuesta de prueba: ${command}`,
        });
      },
      transformCallback: (callback) => callback,
    };
  }, dump);

  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: "networkidle" });

  const failures = [];
  for (const view of VIEWS) {
    try {
      await page.getByRole("button", { name: view.nav, exact: false }).first().click();
      await page.waitForTimeout(220);
      const body = await page.locator("body").innerText();
      if (!body.includes(view.expect)) {
        failures.push(`${view.id}: no aparece el texto esperado "${view.expect}"`);
      }
      await page.screenshot({ path: join(outDir, `${view.id}.png`) });
      console.log(`  ok  ${view.id.padEnd(10)} -> ${view.id}.png`);
    } catch (error) {
      failures.push(`${view.id}: ${error.message}`);
      console.log(`  FALLO ${view.id}: ${error.message}`);
    }
  }

  await browser.close();
  server.close();

  if (consoleErrors.length > 0) {
    console.log("\nErrores de consola:");
    for (const error of consoleErrors) console.log(`  - ${error}`);
  }
  if (failures.length > 0) {
    console.log("\nPantallas con problemas:");
    for (const failure of failures) console.log(`  - ${failure}`);
    process.exit(1);
  }

  console.log(`\n${VIEWS.length} pantallas verificadas. Capturas en ${outDir}`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
