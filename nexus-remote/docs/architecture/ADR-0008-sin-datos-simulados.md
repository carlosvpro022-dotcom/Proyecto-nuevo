# ADR-0008 · Nada de datos simulados en la interfaz

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

En un proyecto por etapas es tentador rellenar las pantallas que aún no
funcionan con datos de ejemplo, para que el producto "se vea completo". El
problema aparece después: nadie recuerda qué era real y qué era de mentira, y
una pantalla con datos falsos es indistinguible de una rota.

## Decisión

La interfaz **nunca muestra datos inventados**. Las reglas concretas:

1. Una pantalla cuya funcionalidad no existe muestra un componente `Pending`
   que dice en qué etapa llega, qué falta exactamente y de qué depende.
2. Un backend sin implementación devuelve un error explícito
   (`UnsupportedPlatform`), no una lista vacía ni un valor por defecto.
3. Si el panel se abre sin backend detrás, muestra "sin conexión con el host" y
   explica cómo arrancarlo. No carga datos de ejemplo.
4. Un campo que todavía no se puede averiguar —como si la sesión de Windows
   está bloqueada— devuelve su valor neutro y está documentado como pendiente,
   con una prueba que lo recuerda.
5. La verificación visual del panel (`scripts/verificar-panel.mjs`) usa datos
   **reales** producidos por el backend (`cargo run --example volcado_ui`), no
   un fichero de ejemplo escrito a mano.

## Alternativas descartadas

- **Pantallas con datos de ejemplo.** Enseñan mejor el diseño, pero inducen a
  error sobre el estado real del proyecto.
- **Ocultar las pantallas no implementadas.** Se pierde la visión de a dónde va
  el producto, y la barra lateral cambiaría de forma en cada etapa.

## Consecuencias

- El panel en la etapa 1 tiene cuatro pantallas con contenido real (Panel, Mis
  dispositivos, Conexión remota, Seguridad), dos funcionales (Configuración y
  el catálogo de permisos) y tres que declaran su estado pendiente.
- La prueba `the_feature_matrix_is_consistent_with_the_stage_plan` falla si
  alguna función de una etapa posterior se marca como lista.
- El indicador `E2`, `E3`, `E5` junto a cada entrada de la barra lateral señala
  la etapa en que esa pantalla cobrará vida.
