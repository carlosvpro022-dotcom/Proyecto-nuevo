import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri sirve el panel desde el disco, no desde un servidor, asi que las rutas
// deben ser relativas. El puerto fijo es el que espera `tauri.conf.json`.
export default defineConfig({
  plugins: [react()],
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Windows 10 y 11 llevan WebView2 sobre Chromium reciente: no hace falta
    // transpilar a navegadores antiguos.
    target: "chrome120",
    sourcemap: false,
  },
  server: {
    port: 5173,
    strictPort: true,
  },
  clearScreen: false,
});
