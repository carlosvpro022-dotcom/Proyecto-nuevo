import { WebSocketServer, WebSocket } from 'ws';
import type { Server } from 'node:http';
import { env } from '../env.js';

export type RealtimeEvent =
  | { type: 'message:new'; payload: unknown }
  | { type: 'message:update'; payload: unknown }
  | { type: 'conversation:update'; payload: unknown }
  | { type: 'contact:update'; payload: unknown }
  | { type: 'ping'; payload: { at: number } };

const clients = new Set<WebSocket>();

export function attachRealtime(server: Server): void {
  const wss = new WebSocketServer({ noServer: true });

  server.on('upgrade', (req, socket, head) => {
    const url = new URL(req.url ?? '/', `http://${req.headers.host ?? 'localhost'}`);
    if (url.pathname !== '/ws') {
      socket.destroy();
      return;
    }
    if (env.apiKey && url.searchParams.get('key') !== env.apiKey) {
      socket.write('HTTP/1.1 401 Unauthorized\r\n\r\n');
      socket.destroy();
      return;
    }
    wss.handleUpgrade(req, socket, head, (ws) => wss.emit('connection', ws, req));
  });

  wss.on('connection', (ws: WebSocket) => {
    clients.add(ws);
    let alive = true;
    ws.on('pong', () => {
      alive = true;
    });
    const beat = setInterval(() => {
      if (!alive) {
        ws.terminate();
        return;
      }
      alive = false;
      ws.ping();
    }, 30_000);
    ws.on('close', () => {
      clients.delete(ws);
      clearInterval(beat);
    });
    ws.on('error', () => {
      clients.delete(ws);
      clearInterval(beat);
    });
  });
}

export function broadcast(event: RealtimeEvent): void {
  const data = JSON.stringify(event);
  for (const ws of clients) {
    if (ws.readyState === ws.OPEN) {
      try {
        ws.send(data);
      } catch {
        clients.delete(ws);
      }
    }
  }
}

export function connectedClients(): number {
  return clients.size;
}
