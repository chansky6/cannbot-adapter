import type { UsageRow } from "./ipc";

/// Color palette for model segments in the histogram.
export const MODEL_COLORS = [
  "#559bf2",
  "#32bd76",
  "#f2a552",
  "#b06ce6",
  "#e66a8a",
  "#5bc8d9",
  "#a0a84a",
  "#8a8f9c",
];

export interface UsageByDateRow {
  date: string;
  requests: number;
  input: number;
  output: number;
  models: number;
}

export interface UsageByModelRow {
  model: string;
  requests: number;
  input: number;
  output: number;
  days: number;
}

export function usageByDate(rows: UsageRow[]): UsageByDateRow[] {
  return [
    ...rows
      .reduce((map, r) => {
        const cur = map.get(r.date) ?? { date: r.date, requests: 0, input: 0, output: 0, models: 0 };
        cur.requests += r.requests;
        cur.input += r.input_tokens;
        cur.output += r.output_tokens;
        cur.models += 1;
        map.set(r.date, cur);
        return map;
      }, new Map<string, UsageByDateRow>())
      .values(),
  ].sort((a, b) => b.date.localeCompare(a.date));
}

export function usageByModel(rows: UsageRow[]): UsageByModelRow[] {
  return [
    ...rows
      .reduce((map, r) => {
        const cur = map.get(r.model) ?? { model: r.model, requests: 0, input: 0, output: 0, days: 0 };
        cur.requests += r.requests;
        cur.input += r.input_tokens;
        cur.output += r.output_tokens;
        cur.days += 1;
        map.set(r.model, cur);
        return map;
      }, new Map<string, UsageByModelRow>())
      .values(),
  ].sort((a, b) => b.output + b.input - (a.output + a.input));
}

export function usageTotal(rows: UsageRow[]) {
  return {
    requests: rows.reduce((s, r) => s + r.requests, 0),
    input: rows.reduce((s, r) => s + r.input_tokens, 0),
    output: rows.reduce((s, r) => s + r.output_tokens, 0),
  };
}

export interface UsageDay {
  date: string;
  label: string;
  segments: { model: string; tokens: number }[];
  total: number;
}

/// Daily series for the histogram: one bar per day (zero-filled), each bar
/// stacked by model with per-model token totals. Dates are formatted in local
/// time — toISOString() would shift back to UTC and misattribute the day in
/// non-UTC timezones.
export function usageDaily(rows: UsageRow[]): { days: UsageDay[]; colorOf: Map<string, string> } {
  const days: UsageDay[] = [];
  const colorOf = new Map<string, string>();
  if (rows.length === 0) return { days, colorOf };

  // model -> color index (stable across days, ordered by total desc)
  const modelTotals = new Map<string, number>();
  for (const r of rows) {
    modelTotals.set(r.model, (modelTotals.get(r.model) ?? 0) + r.input_tokens + r.output_tokens);
  }
  const models = [...modelTotals.entries()].sort((a, b) => b[1] - a[1]).map(([m]) => m);
  for (let i = 0; i < models.length; i++) {
    colorOf.set(models[i], MODEL_COLORS[i % MODEL_COLORS.length]);
  }

  // date -> model -> tokens
  const byDate = new Map<string, Map<string, number>>();
  for (const r of rows) {
    let m = byDate.get(r.date);
    if (!m) {
      m = new Map();
      byDate.set(r.date, m);
    }
    m.set(r.model, (m.get(r.model) ?? 0) + r.input_tokens + r.output_tokens);
  }
  const dates = [...byDate.keys()].sort();
  const start = new Date(dates[0] + "T00:00:00");
  const end = new Date(dates[dates.length - 1] + "T00:00:00");
  const pad = (n: number) => String(n).padStart(2, "0");
  for (let d = new Date(start); d <= end; d.setDate(d.getDate() + 1)) {
    const iso = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
    const dayModels = byDate.get(iso);
    const segments = dayModels ? [...dayModels.entries()].map(([model, tokens]) => ({ model, tokens })) : [];
    days.push({
      date: iso,
      label: `${d.getMonth() + 1}/${d.getDate()}`,
      segments,
      total: segments.reduce((s, x) => s + x.tokens, 0),
    });
  }
  return { days, colorOf };
}
