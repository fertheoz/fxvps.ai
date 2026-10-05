export function originOf(url: string | undefined | null): string | null;
export function buildPolicy(hashes: string[], apiUrl?: string | (string | undefined)[]): string;
export function applyCsp(html: string, apiUrl?: string | (string | undefined)[]): string;
