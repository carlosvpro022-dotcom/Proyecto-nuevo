import { mkdirSync, copyFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';

const assets = ['db/schema.sql'];
for (const rel of assets) {
  const to = resolve('dist', rel);
  mkdirSync(dirname(to), { recursive: true });
  copyFileSync(resolve('src', rel), to);
  console.log('copiado', rel);
}
