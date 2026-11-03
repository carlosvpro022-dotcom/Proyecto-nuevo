/**
 * Pantallas que todavia no tienen funcionalidad detras.
 *
 * Cada una dice en que etapa llega, que falta exactamente y de que depende.
 * Es la alternativa honesta a una maqueta con datos de ejemplo: la persona ve
 * el sitio que ocupara la funcion, pero no puede confundirla con algo que ya
 * funciona.
 */

import type { FeatureStatus } from "../api";
import { Pending } from "../components/ui";

function detailFor(features: FeatureStatus[], key: string, fallback: string): string {
  return features.find((f) => f.key === key)?.detail ?? fallback;
}

function stageFor(features: FeatureStatus[], key: string, fallback: number): number {
  return features.find((f) => f.key === key)?.stage ?? fallback;
}

export function Files({ features }: { features: FeatureStatus[] }) {
  return (
    <div className="stack">
      <Pending
        title="Transferencia de archivos"
        stage={stageFor(features, "files", 5)}
        detail={detailFor(
          features,
          "files",
          "Falta mover bytes por el canal de datos.",
        )}
        dependsOn="Etapa 3 (canal de datos de WebRTC)"
      />
    </div>
  );
}

export function History({ features }: { features: FeatureStatus[] }) {
  return (
    <div className="stack">
      <Pending
        title="Historial de sesiones"
        stage={stageFor(features, "signaling", 3)}
        detail={
          "La tabla de sesiones y su registro ya existen en la base de datos local, " +
          "con dispositivo, duracion y motivo de cierre. Esta vacia porque todavia no " +
          "se puede abrir ninguna sesion remota."
        }
        dependsOn="Etapa 3 (primera sesion remota establecida)"
      />
    </div>
  );
}
