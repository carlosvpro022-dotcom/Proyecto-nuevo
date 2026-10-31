// Genera los archivos de ejemplo (PNG y PDF reales) que usa el seed de demostración.
import zlib from 'node:zlib';
import fs from 'node:fs';
import path from 'node:path';

function crc32(buf) {
  let c, crc = 0xffffffff;
  for (let n = 0; n < buf.length; n++) {
    c = (crc ^ buf[n]) & 0xff;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    crc = c ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

/** PNG RGB de un degradado, suficiente para ver una foto real en la burbuja. */
export function makePng(width, height, colorFn) {
  const raw = Buffer.alloc((width * 3 + 1) * height);
  let o = 0;
  for (let y = 0; y < height; y++) {
    raw[o++] = 0; // filtro "none"
    for (let x = 0; x < width; x++) {
      const [r, g, b] = colorFn(x / width, y / height);
      raw[o++] = r;
      raw[o++] = g;
      raw[o++] = b;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // profundidad
  ihdr[9] = 2; // color RGB
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** PDF mínimo pero válido, para que la descarga del documento funcione. */
export function makePdf(title, lines) {
  const content = lines
    .map((l, i) => `BT /F1 ${i === 0 ? 18 : 11} Tf 60 ${740 - i * 26} Td (${l.replace(/[()\\]/g, '')}) Tj ET`)
    .join('\n');
  const objs = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    `<< /Title (${title}) >>`,
  ];
  let pdf = '%PDF-1.4\n';
  const offsets = [];
  objs.forEach((body, i) => {
    offsets.push(pdf.length);
    pdf += `${i + 1} 0 obj\n${body}\nendobj\n`;
  });
  const xref = pdf.length;
  pdf += `xref\n0 ${objs.length + 1}\n0000000000 65535 f \n`;
  for (const off of offsets) pdf += `${String(off).padStart(10, '0')} 00000 n \n`;
  pdf += `trailer\n<< /Size ${objs.length + 1} /Root 1 0 R /Info 6 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return Buffer.from(pdf, 'latin1');
}

export function writeDemoFiles(mediaDir) {
  const inbound = path.join(mediaDir, 'inbound');
  const outbound = path.join(mediaDir, 'outbound');
  fs.mkdirSync(inbound, { recursive: true });
  fs.mkdirSync(outbound, { recursive: true });

  fs.writeFileSync(
    path.join(inbound, 'demo-nomina.png'),
    makePng(520, 340, (u, v) => {
      // Simula el recorte de una nómina: papel claro con renglones.
      const line = Math.sin(v * 46) > 0.86 ? 40 : 0;
      const head = v < 0.17 ? 28 : 0;
      const base = 236 - u * 10;
      return [base - line - head, base - line - head + 2, base - line - head - 4];
    })
  );

  fs.writeFileSync(
    path.join(inbound, 'demo-coche.png'),
    makePng(560, 360, (u, v) => {
      const sky = [70 + v * 60, 110 + v * 70, 150 + v * 60];
      const road = [58, 58, 62];
      const car = v > 0.52 && v < 0.78 && u > 0.2 && u < 0.8;
      if (car) return [190, 60, 55];
      return v > 0.8 ? road : sky;
    })
  );

  fs.writeFileSync(
    path.join(outbound, 'demo-condiciones.pdf'),
    makePdf('Condiciones de financiacion', [
      'Torque Financiero',
      '',
      'Propuesta de financiacion - vehiculo de ocasion',
      '',
      'Importe solicitado:        14.500 EUR',
      'Plazo:                     60 meses',
      'TIN:                       6,95 %',
      'TAE:                       7,42 %',
      'Cuota mensual estimada:    287,40 EUR',
      '',
      'Documentacion necesaria:',
      '  - DNI en vigor por ambas caras',
      '  - Dos ultimas nominas',
      '  - Justificante de domicilio',
      '',
      'Oferta sujeta a estudio y aprobacion. Documento de ejemplo.',
    ])
  );

  return {
    nomina: 'inbound/demo-nomina.png',
    coche: 'inbound/demo-coche.png',
    condiciones: 'outbound/demo-condiciones.pdf',
  };
}
