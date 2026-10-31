import { useEffect, useState } from 'react';
import { Check, Search } from 'lucide-react';
import { api } from '../lib/api';
import { useStore } from '../lib/store';
import { Modal } from './Modal';
import type { CatalogSummary, Product } from '../types';

export function ProductPicker({
  conversationId,
  onClose,
}: {
  conversationId: number;
  onClose: () => void;
}) {
  const [catalogs, setCatalogs] = useState<CatalogSummary[]>([]);
  const [catalogId, setCatalogId] = useState<string>('');
  const [products, setProducts] = useState<Product[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [query, setQuery] = useState('');
  const [body, setBody] = useState('Estos son nuestros productos:');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const toast = useStore((s) => s.toast);

  useEffect(() => {
    (async () => {
      try {
        const res = await api.catalogs();
        setCatalogs(res.catalogs);
        if (res.catalogs[0]) setCatalogId(res.catalogs[0].id);
        else setError('No hay ningún catálogo conectado a esta cuenta de WhatsApp Business.');
      } catch (err) {
        setError((err as Error).message);
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  useEffect(() => {
    if (!catalogId) return;
    setLoading(true);
    api
      .products(catalogId, query || undefined)
      .then((res) => {
        setProducts(res.products);
        setError(null);
      })
      .catch((err) => setError((err as Error).message))
      .finally(() => setLoading(false));
  }, [catalogId, query]);

  function toggle(retailerId: string) {
    setSelected((s) =>
      s.includes(retailerId) ? s.filter((r) => r !== retailerId) : s.length < 30 ? [...s, retailerId] : s
    );
  }

  async function send() {
    if (selected.length === 0) {
      toast('Selecciona al menos un producto', 'error');
      return;
    }
    setSending(true);
    try {
      await api.sendProducts(conversationId, catalogId, selected, body);
      toast(`${selected.length} producto(s) enviados`, 'success');
      onClose();
    } catch (err) {
      toast((err as Error).message, 'error');
    } finally {
      setSending(false);
    }
  }

  return (
    <Modal title="Enviar productos del catálogo" onClose={onClose} wide>
      {error && !products.length && (
        <div className="rounded-lg bg-amber-500/10 p-4 text-sm text-amber-200">
          <p>{error}</p>
          <p className="mt-2 text-xs text-amber-200/80">
            El catálogo se crea en el Administrador de Comercio de Meta y se conecta a la cuenta de WhatsApp
            Business. El token necesita el permiso <code className="rounded bg-black/30 px-1">catalog_management</code>.
          </p>
        </div>
      )}

      <div className="space-y-3">
        {catalogs.length > 1 && (
          <select
            value={catalogId}
            onChange={(e) => setCatalogId(e.target.value)}
            className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm"
          >
            {catalogs.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name} {c.product_count ? `(${c.product_count})` : ''}
              </option>
            ))}
          </select>
        )}

        <div className="flex items-center gap-2 rounded-lg bg-wa-surface-2 px-3">
          <Search className="h-4 w-4 text-wa-muted" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Buscar producto por nombre o SKU"
            className="w-full bg-transparent py-2 text-sm placeholder:text-wa-muted"
          />
        </div>

        {loading && <p className="py-6 text-center text-sm text-wa-muted">Cargando catálogo…</p>}

        <div className="grid max-h-72 grid-cols-2 gap-2 overflow-y-auto pr-1 sm:grid-cols-3">
          {products.map((p) => {
            const on = selected.includes(p.retailer_id);
            return (
              <button
                key={p.id}
                onClick={() => toggle(p.retailer_id)}
                className={`relative overflow-hidden rounded-lg border text-left transition ${
                  on ? 'border-wa-accent bg-wa-accent/10' : 'border-wa-border bg-wa-surface hover:bg-wa-surface-2'
                }`}
              >
                {p.image_url && (
                  <img src={p.image_url} alt="" loading="lazy" className="h-24 w-full object-cover" />
                )}
                <div className="p-2">
                  <p className="truncate text-xs">{p.name}</p>
                  <p className="text-[11px] text-wa-muted">{p.price}</p>
                  <p className="truncate text-[10px] text-wa-muted/70">{p.retailer_id}</p>
                </div>
                {on && (
                  <span className="absolute top-1.5 right-1.5 rounded-full bg-wa-accent p-1 text-black">
                    <Check className="h-3 w-3" />
                  </span>
                )}
              </button>
            );
          })}
        </div>

        <label className="block">
          <span className="mb-1 block text-xs text-wa-muted">Mensaje que acompaña a los productos</span>
          <input
            value={body}
            onChange={(e) => setBody(e.target.value)}
            className="w-full rounded-lg bg-wa-surface-2 px-3 py-2 text-sm"
          />
        </label>

        <button
          onClick={() => void send()}
          disabled={sending || selected.length === 0}
          className="w-full rounded-lg bg-wa-accent py-2.5 text-sm font-medium text-black transition hover:brightness-110 disabled:opacity-40"
        >
          {sending ? 'Enviando…' : `Enviar ${selected.length || ''} producto${selected.length === 1 ? '' : 's'}`}
        </button>
      </div>
    </Modal>
  );
}
