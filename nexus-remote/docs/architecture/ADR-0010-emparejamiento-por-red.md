# ADR-0010 · El servidor no retransmite códigos de emparejamiento

**Estado:** aceptada · **Fecha:** 2026-11-03

## Contexto

El emparejamiento funciona así: el host muestra un código temporal (ocho
caracteres de un alfabeto de 32, unos 40 bits) y el dispositivo nuevo lo
presenta para quedar autorizado. La lógica está terminada y probada desde la
etapa 1: `PairingSession` guarda el código como hash Argon2id, nunca en claro,
con caducidad e intentos limitados.

Lo que faltaba era transportarlo. El protocolo tiene desde la etapa 1 un mensaje
`RedeemPairingCode` que lleva el código y la clave pública del dispositivo nuevo,
y que el servidor reenviaría al host.

Al implementarlo apareció el problema: **el código viajaría en claro a través del
servidor**, que es exactamente a quien la ADR-0009 deja de considerar de
confianza. Un servidor comprometido vería el código, y con el código puede
emparejarse él mismo como si fuera el iPad del usuario. A partir de ahí tiene
acceso legítimo y permanente al PC, y ninguna de las defensas posteriores
—incluida la atadura de sesión— lo detecta, porque ya es un dispositivo
autorizado.

Mandar un verificador en lugar del código tampoco basta por sí solo: con 40 bits
de entropía, un servidor que vea un HMAC puede probar las 2⁴⁰ combinaciones y
recuperar el código.

## Decisión

**No se implementa el relevo de emparejamiento en esta etapa.** El canal
rechaza `RedeemPairingCode` con el código `pairing_relay_unavailable`, y
`GET /v1/info` publica `pairing_relay: false` para que un cliente lo sepa antes
de intentarlo.

Rechazarlo explícitamente, y no dejarlo sin implementar en silencio, es
deliberado: un `404` o un mensaje ignorado parecería un fallo de red.

## Lo que falta, y cómo se piensa hacer

Tres piezas, en este orden:

1. **La invitación lleva la clave pública completa del host.** Ya está hecho en
   esta etapa (`PairingInvite::host_public_key_hex`). Es lo que permite al
   controlador autenticar al host desde el primer momento sin preguntar al
   servidor.

2. **Por la red viaja un verificador Argon2id, no el código.** La invitación
   incluiría la sal del emparejamiento; el dispositivo nuevo derivaría el mismo
   hash que el host tiene guardado y enviaría ése. Un servidor que lo viese
   tendría que ejecutar Argon2id 2⁴⁰ veces para recuperar el código, que con los
   parámetros actuales son del orden de decenas de miles de años de CPU. No es
   una defensa perfecta —un PAKE lo sería— pero es una barrera real y no añade
   criptografía propia.

3. **El host confirma mostrando la huella de la clave del dispositivo nuevo.**
   La persona compara lo que ve en el PC con lo que ve en el iPad. Esto es lo
   que cierra el ataque de verdad: un servidor que hubiese sustituido la clave
   del controlador mostraría una huella distinta en cada pantalla. Es el mismo
   mecanismo de comparación numérica que usan SSH, Signal y el emparejamiento
   de Bluetooth.

Mientras esas tres piezas no estén, el emparejamiento se hace **en la red
local**, donde el servidor propio no está en medio.

## Consecuencias

A favor:

- No se expone un camino por el que un servidor comprometido pueda autorizarse a
  sí mismo.
- El límite queda declarado en el protocolo, en el servidor y en la interfaz, en
  lugar de ser una sorpresa.

En contra:

- Un dispositivo nuevo no se puede emparejar desde fuera de casa todavía. Es una
  limitación real del producto en este punto, y está en la lista de pendientes
  del panel y del README.
