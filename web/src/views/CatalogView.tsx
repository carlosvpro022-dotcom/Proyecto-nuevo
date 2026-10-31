import { useEffect, useState } from 'react';
import { ExternalLink, RefreshCw, Search, ShoppingBag } from 'lucide-react';
import { api } from '../lib/api';
import { useStore } from '../lib/store';
import type { CatalogSummary, Product } from '../types';

export function CatalogView() {
  const [catalogs, setCatalogs] = useState<CatalogSummary[]>([]);
  const [catalogId, setCatalogId] = useState('');
  const [products, setProducts] = useState<Product[]>([]);
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const toast = useStore((s) => s.toast);

  async function loadCatalogs(refresh = false) {
    setLoading(true);
    setError(null);
    try {
      const res = await api.catalogs(refresh);
      setCatalogs(res.catalogs);
      if (res.catalogs[0]) setCatalogId((id) => id || res.catalogs[0].id);
      else setError('No hay ningún catálogo conectado a esta cuenta de WhatsApp Business.');
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadCatalogs();
  }, []);

  useEffect(() => {
    if (!catalogId) return;
    const id = setTimeout(() => {
      setLoading(true);
      api
        .products(catalogId, query || undefined)
        .then((r) => {
          setProducts(r.products);
          setError(null);
        })
        .catch((e) => setError((e as Error).message))
        .finally(() => setLoading(false));
    }, 250);
    return () => clearTimeout(id);
  }, [catalogId, query]);

  const active = catalogs.find((c) => c.id === catalogId);

  return (
    <section className="flex min-w-0 flex-1 flex-col bg-wa-bg">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b border-wa-border bg-wa-surface px-6 py-4">
        <div>
          <h1 className="text-lg">Catálogo</h1>
          <p className="mt-0.5 text-xs text-wa-muted">
            Productos de tu catálogo de comercio conectado a WhatsApp Business.
          </p>
        </div>
        <button
          onClick={() => void loadCatalogs(true)}
          className="flex items-center gap-2 rounded-lg bg-wa-surface-2 px-3 py-2 text-xs text-wa-muted transition hover:text-wa-text"
        >
          <RefreshCw className="h-3.5 w-3.5" /> Recargar
        </button>
      </header>

      <div className="flex flex-wrap items-center gap-3 border-b border-wa-border px-6 py-3">
        {catalogs.length > 0 && (
          <select
            value={catalogId}
            onChange={(e) => setCatalogId(e.target.value)}
            className="rounded-lg bg-wa-surface px-3 py-2 text-sm"
          >
            {catalogs.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        )}
        <div className="flex min-w-60 flex-1 items-center gap-2 rounded-lg bg-wa-surface px-3">
          <Search className="h-4 w-4 text-wa-muted" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Buscar producto por nombre, SKU o descripción"
            className="w-full bg-transparent py-2 text-sm placeholder:text-wa-muted"
          />
        </div>
        {active?.product_count !== undefined && (
          <span className="text-xs text-wa-muted">{active.product_count} productos en total</span>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-6">
        {error && (
          <div className="mx-auto max-w-xl rounded-lg bg-amber-500/10 p-5 text-sm text-amber-200">
            <p className="font-medium">{error}</p>
            <ul className="mt-3 list-disc space-y-1.5 pl-5 text-xs text-amber-200/85">
              <li>
                Crea el catálogo en el <strong>Administrador de Comercio</strong> de Meta y conéctalo a tu cuenta de
                WhatsApp Business.
              </li>
              <li>
                El token necesita los permisos <code className="rounded bg-black/30 px-1">catalog_management</code> y{' '}
                <code className="rounded bg-black/30 px-1">whatsapp_business_management</code>.
              </li>
              <li>
                Comprueba que <code className="rounded bg-black/30 px-1">WHATSAPP_BUSINESS_ACCOUNT_ID</code> es
                correcto en el archivo <code className="rounded bg-black/30 px-1">.env</code>.
              </li>
            </ul>
          </div>
        )}

        {loading && products.length === 0 && !error && <p className="text-sm text-wa-muted">Cargando catálogo…</p>}

        {!loading && !error && products.length === 0 && (
          <div className="mx-auto max-w-md rounded-lg bg-wa-panel p-8 text-center">
            <ShoppingBag className="mx-auto h-10 w-10 text-wa-muted" />
            <p className="mt-3 text-sm text-wa-muted">
              {query ? 'Ningún producto coincide con la búsqueda.' : 'Este catálogo no tiene productos.'}
            </p>
          </div>
        )}

        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
          {products.map((p) => (
            <article key={p.id} className="overflow-hidden rounded-lg bg-wa-panel">
              {p.image_url ? (
                <img src={p.image_url} alt="" loading="lazy" className="h-40 w-full object-cover" />
              ) : (
                <div className="flex h-40 items-center justify-center bg-wa-surface-2">
                  <ShoppingBag className="h-8 w-8 text-wa-muted" />
                </div>
              )}
              <div className="space-y-1.5 p-4">
                <h3 className="truncate text-sm font-medium" title={p.name}>
                  {p.name}
                </h3>
                <p className="text-sm text-wa-accent">{p.price}</p>
                {p.description && <p className="line-clamp-2 text-xs text-wa-muted">{p.description}</p>}
                <div className="flex items-center justify-between pt-1.5 text-[11px] text-wa-muted">
                  <span className="truncate" title={p.retailer_id}>
                    SKU: {p.retailer_id}
                  </span>
                  <span
                    className={
                      p.availability === 'in stock' || p.availability === 'in_stock'
                        ? 'text-wa-accent'
                        : 'text-amber-400'
                    }
                  >
                    {p.availability}
                  </span>
                </div>
                {p.url && (
                  <a
                    href={p.url}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex items-center gap-1.5 pt-1 text-xs text-wa-blue hover:underline"
                  >
                    Ver ficha <ExternalLink className="h-3 w-3" />
                  </a>
                )}
              </div>
            </article>
          ))}
        </div>

        {products.length > 0 && (
          <p className="mt-6 text-center text-xs text-wa-muted">
            Para enviar productos a un cliente, abre su chat y usa el botón de adjuntar → Producto del catálogo.
          </p>
        )}
      </div>
    </section>
  );
}
