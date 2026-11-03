# ADR-0004 · Identidad Ed25519 por dispositivo y autenticación mutua

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

El sistema es privado: solo deben entrar los dispositivos del dueño. Una
contraseña compartida no sirve, porque se filtra, no se puede revocar por
dispositivo y no identifica quién entró.

## Decisión

Cada instalación —el host y cada dispositivo Apple— genera **su propio par de
claves Ed25519** la primera vez que arranca. La clave privada no sale nunca del
dispositivo. Lo que se intercambia al emparejar es solo la pública.

La autenticación es **mutua**: ambos extremos firman una transcripción que
contiene:

```
"NEXUS-REMOTE/v1/mutual-auth" || 0x00 || papel || versión ||
nonce_servidor(32) || nonce_cliente(32) || id_dispositivo(16) || marca(8)
```

Cada campo está ahí por una razón concreta, documentada en
`crates/security/src/challenge.rs`. En particular el **byte de papel** (host o
controlador) impide el ataque de reflexión: sin él, un atacante podría devolver
al host su propia firma y hacerse pasar por el otro extremo.

Bibliotecas: `ed25519-dalek` para firmas, `argon2` para los códigos de
emparejamiento, `sha2` para huellas, `subtle` para comparaciones en tiempo
constante. **No se implementa ninguna primitiva criptográfica propia.**

## Alternativas descartadas

- **Contraseña compartida (modelo AnyDesk clásico).** Se filtra, no se revoca
  por dispositivo y no deja rastro de quién entró.
- **Certificados X.509 con una CA propia.** Funcionaría, pero montar y
  mantener una autoridad certificadora para tres dispositivos es
  desproporcionado, y añade la gestión de caducidades y revocaciones (CRL u
  OCSP) sin ganar nada.
- **TLS con autenticación de cliente en la señalización.** Autenticaría contra
  el servidor, no entre los dispositivos. El servidor es el componente del que
  menos queremos depender.
- **RSA en lugar de Ed25519.** Claves mucho mayores, firmas más lentas y más
  formas de configurarlo mal.

## Decisiones derivadas

- Se usa `verify_strict`, que rechaza claves de orden pequeño y firmas
  maleables. Además esas claves se rechazan **al registrar el dispositivo**,
  no solo al verificar: así un dispositivo con una clave inútil no llega a
  emparejarse.
- El código de emparejamiento son 8 caracteres sobre un alfabeto de 32 sin
  caracteres confundibles: 40 bits. Lo que de verdad lo protege no es la
  entropía sino que vive 5 minutos y admite 5 intentos.
- El código se guarda derivado con Argon2id, nunca en claro, y la derivación
  incluye el identificador del emparejamiento para que un código observado en
  uno no sirva en otro.

## Consecuencias

- **Pendiente para producción:** la clave privada del host se guarda en un
  archivo con permisos restringidos. En Windows eso no protege de otro proceso
  del mismo usuario. Hay que cifrarla con DPAPI (`CryptProtectData` con ámbito
  de usuario) antes de considerar el host listo. Está recogido como tarea de la
  etapa 6 y avisado en la pantalla de Configuración del panel.
- Si se pierde el archivo de clave del host, todos los emparejamientos quedan
  inservibles y hay que repetirlos. Por eso, si el archivo existe pero está
  corrupto, el host **falla al arrancar** en lugar de generar una identidad
  nueva en silencio.
