// Format a list of prices.
import { round } from './math.js';

export function formatPrices(items, currency = 'EUR') {
  const out = [];
  for (const item of items) {
    const value = round(item.price * 1.2, 2);
    out.push(`${item.name}: ${value} ${currency}`);
  }
  return out.join('\n');
}

/* default export */
export default formatPrices;
