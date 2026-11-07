import { defineConfig } from "vite";

export default defineConfig({
  // Rutas relativas: el servidor de senalizacion sirve estos archivos bajo una
  // ruta que no tiene por que ser la raiz.
  base: "./",
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
