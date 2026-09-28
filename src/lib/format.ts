export function fmtTime(unix: number | null): string {
  if (!unix) return "-";
  return new Date(unix * 1000).toLocaleTimeString();
}

export function fmtK(n: number): string {
  if (n >= 1_000_000) {
    return `${(n / 1_000_000).toFixed(n % 1_000_000 === 0 ? 0 : 1)}M`;
  }
  if (n >= 1_000) {
    return `${Math.round(n / 1_000)}K`;
  }
  return String(n);
}

export function fmtTokens(n: number): string {
  return n.toLocaleString();
}
