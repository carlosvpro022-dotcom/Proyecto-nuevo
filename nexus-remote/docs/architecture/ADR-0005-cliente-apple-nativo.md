# ADR-0005 · Cliente Apple nativo en SwiftUI

**Estado:** aceptada · **Fecha:** 2026-11-03 · **Implementación:** etapa 4

## Contexto

El cliente es lo que se usa a diario desde el iPhone y el iPad. Tiene que
decodificar vídeo con poca latencia, traducir gestos táctiles a ratón y teclado,
y sentirse como una aplicación de iPad, no como una web metida en un marco.

## Decisión

Aplicación **nativa en Swift y SwiftUI**, con `WebRTC.framework` para el
transporte y `VideoToolbox` para la decodificación por hardware.

La clave privada del dispositivo se guarda en el **Keychain** con
`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`: no sale del dispositivo, no se
sincroniza con iCloud y no es accesible con el teléfono bloqueado.

## Alternativas descartadas

- **PWA o aplicación web.** Safari en iOS no da acceso al teclado físico
  completo, la gestión de pantalla completa es limitada, no hay decodificación
  por hardware controlable y no se puede guardar una clave en el Secure
  Enclave. Para una herramienta de control remoto el resultado sería mediocre.
- **React Native o Flutter.** Habría que escribir igualmente el puente nativo
  para WebRTC y VideoToolbox, que es la parte difícil, y encima se añade una
  capa que complica la depuración de latencia.
- **Catalyst desde una app de macOS.** No hay app de macOS, y los gestos
  táctiles del iPad son justamente lo que hay que diseñar con cuidado.

## Sobre compartir los contratos con Rust

El cliente **no** está en el workspace de Cargo. Hay dos caminos para que no
diverja del protocolo, y la decisión se toma al empezar la etapa 4:

1. **Tipos Swift escritos a mano**, con una prueba en CI que compara el JSON
   generado por Swift con el que espera `nexus-core`. Más simple, pero hay que
   mantener dos definiciones.
2. **`nexus-core` compilado a una biblioteca estática** para
   `aarch64-apple-ios` y expuesto por FFI con `uniffi` o `cbindgen`. Una sola
   definición, a cambio de complicar la compilación en Xcode.

La preferencia inicial es la opción 1 para el protocolo de señalización, que es
estable y pequeño, y la opción 2 para la validación de entrada, donde duplicar
las reglas sí sería peligroso.

## Consecuencias

- **Hace falta un Mac con Xcode.** No se puede compilar ni probar el cliente
  desde Linux ni desde Windows. Es una dependencia física del proyecto.
- Hace falta una cuenta de desarrollador de Apple para instalar en dispositivos
  propios. Con una cuenta gratuita se puede, pero la firma caduca cada 7 días;
  con una de pago, cada año.
- Como la aplicación es privada y no va a la App Store, la distribución es por
  Xcode, TestFlight o distribución ad hoc.
