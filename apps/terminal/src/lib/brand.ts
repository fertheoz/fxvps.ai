import { create } from 'zustand';
import { clientApiBase } from '../api/clientApi';

export interface Brand {
  name: string;
  brandColor: string;
  logoUrl: string;
  supportEmail: string;
}

const DEFAULT: Brand = { name: 'fxvps.ai', brandColor: '', logoUrl: '', supportEmail: '' };

export const useBrand = create<Brand>(() => DEFAULT);

/** White label: the tenant that owns this hostname names, colours and logos the terminal. */
export async function loadBrand(): Promise<void> {
  const base = clientApiBase();
  if (!base) return;
  try {
    const r = await fetch(`${base}/brand?host=${encodeURIComponent(window.location.hostname)}`);
    if (!r.ok) return;
    const b = (await r.json()) as Partial<Brand>;
    const brand: Brand = {
      name: b.name || DEFAULT.name,
      brandColor: /^#[0-9a-fA-F]{6}$/.test(b.brandColor ?? '') ? b.brandColor! : '',
      logoUrl: (b.logoUrl ?? '').startsWith('https://') ? b.logoUrl! : '',
      supportEmail: b.supportEmail ?? '',
    };
    useBrand.setState(brand);
    document.title = `${brand.name} Terminal`;
    if (brand.brandColor) document.documentElement.style.setProperty('--accent', brand.brandColor);
  } catch {
    /* offline or no gateway: keep the defaults */
  }
}
