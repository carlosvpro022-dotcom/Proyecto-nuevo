# Despliegue

La guía está en
[`docs/deployment/despliegue.md`](../../docs/deployment/despliegue.md).

Los archivos del despliegue están en [`../docker/`](../docker/):

| Archivo | Qué es |
| --- | --- |
| `compose.yaml` | Los tres servicios: proxy, señalización y Coturn |
| `Dockerfile.signaling` | Imagen del servidor, en dos etapas y sin root |
| `Caddyfile` | Proxy inverso con TLS automático |
| `entrypoint-coturn.sh` | Genera la configuración de Coturn y comprueba el secreto |
| `.env.example` | Todas las variables, documentadas una a una |

Esta carpeta queda reservada para lo que llegue en la etapa 6: manifiestos de
systemd para quien prefiera no usar Docker, y los guiones de empaquetado del
instalador de Windows.
