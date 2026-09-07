// Marcadores do protocolo só valem em linhas próprias, fora de exemplos de código.
function markers(text: string): { start: number; end: number; close: boolean }[] {
  const found: { start: number; end: number; close: boolean }[] = [];
  let offset = 0;
  let fence = '';
  for (const line of text.split('\n')) {
    const delimiter = line.match(/^ {0,3}(`{3,}|~{3,})/);
    if (delimiter) {
      const value = delimiter[1];
      if (!fence) fence = value;
      else if (value[0] === fence[0] && value.length >= fence.length) fence = '';
    } else if (!fence && /^\s*<\/?proposed_plan>\s*$/.test(line)) {
      found.push({ start: offset, end: offset + line.length, close: line.includes('</') });
    }
    offset += line.length + 1;
  }
  return found;
}

export function planDisplayText(text: string): string {
  for (const marker of markers(text).reverse()) {
    text = text.slice(0, marker.start) + text.slice(marker.end);
  }
  return text;
}

export function proposedPlan(text: string): string | null {
  const found = markers(text);
  const close = found.findLast((marker) => marker.close);
  if (!close) return null;
  const open = found.findLast((marker) => !marker.close && marker.start < close.start);
  return text.slice(open?.end ?? 0, close.start).trim() || null;
}
