import type { BookGlossary, GlossaryEntry } from './types';
export const MAX_GLOSSARY_ENTRIES = 10000;
export const MAX_GLOSSARY_BYTES = 4 * 1024 * 1024;
export const MAX_GLOSSARY_FILE_BYTES = 2 * MAX_GLOSSARY_BYTES + 7 * MAX_GLOSSARY_ENTRIES + 18;
export const emptyGlossary = (): BookGlossary => ({
  enabled: false,
  autoDetect: false,
  automaticSources: [],
  entries: [],
  deeplGlossaryId: '',
  revision: 0,
});
export function entryErrors(entries: GlossaryEntry[]): string[] {
  const counts = new Map<string, number>();
  for (const e of entries) counts.set(e.source.trim(), (counts.get(e.source.trim()) || 0) + 1);
  return entries.map((e) =>
    !e.source.trim() || !e.target.trim()
      ? 'Enter both a source term and a translation.'
      : e.source.includes('\0') || e.target.includes('\0')
        ? 'Glossary terms cannot contain null characters.'
        : counts.get(e.source.trim())! > 1
          ? 'Each source term must appear only once.'
          : '',
  );
}
export function normalizeGlossary(value: BookGlossary): BookGlossary {
  if (value.entries.length > MAX_GLOSSARY_ENTRIES) throw Error('Glossary is limited to 10,000 entries.');
  const error = entryErrors(value.entries).find(Boolean);
  if (error) throw Error(error);
  const next = structuredClone(value);
  next.entries = next.entries.map((e) => ({ source: e.source.trim(), target: e.target.trim() }));
  if (
    new TextEncoder().encode(next.entries.map((e) => e.source + e.target).join('')).length >
    MAX_GLOSSARY_BYTES
  )
    throw Error('Glossary exceeds the 4 MiB limit.');
  next.automaticSources = next.automaticSources.filter((s) =>
    next.entries.some((e) => e.source === s),
  );
  next.deeplGlossaryId = next.deeplGlossaryId.trim();
  if (
    new TextEncoder().encode(next.deeplGlossaryId).length > 1024 ||
    /\p{Cc}/u.test(next.deeplGlossaryId)
  )
    throw Error('Invalid hosted glossary ID.');
  return next;
}
export function mergeTerms(existing: GlossaryEntry[], incoming: GlossaryEntry[], replace: boolean) {
  const entries = existing.map((e) => ({ source: e.source.trim(), target: e.target.trim() }));
  let added = 0,
    identical = 0,
    conflicts = 0;
  const positions = new Map(entries.map((e, i) => [e.source, i]));
  for (const term of incoming) {
    const e = { source: term.source.trim(), target: term.target.trim() },
      index = positions.get(e.source);
    if (index === undefined) {
      positions.set(e.source, entries.length);
      entries.push(e);
      added++;
    } else if (entries[index].target === e.target) identical++;
    else {
      conflicts++;
      if (replace) entries[index] = e;
    }
  }
  return { entries, added, identical, conflicts };
}
/** Browser preview counterpart of the native strict two-column parser. */
export function parseGlossary(text: string, format: string): GlossaryEntry[] {
  if (new TextEncoder().encode(text).length > MAX_GLOSSARY_FILE_BYTES)
    throw Error('Glossary file exceeds the encoded size limit.');
  const delimiter = format === 'csv' ? ',' : format === 'tsv' ? '\t' : '';
  if (!delimiter) throw Error('Choose a CSV or TSV file.');
  text = text.replace(/^\uFEFF+/, '');
  let row: string[] = [], field = '', quoted = false, closed = false, firstRow = true, bytes = 0;
  const entries: GlossaryEntry[] = [];
  const finishRow = () => {
    if (row.length !== 2)
      throw Error('Glossary files must contain exactly two columns: source and target.');
    const header = firstRow && row[0].trim().toLowerCase() === 'source' &&
      row[1].trim().toLowerCase() === 'target';
    firstRow = false;
    if (header) return;
    if (entries.length >= MAX_GLOSSARY_ENTRIES) throw Error('Glossary is limited to 10,000 entries.');
    const entry = { source: row[0].trim(), target: row[1].trim() };
    normalizeGlossary({ ...emptyGlossary(), entries: [entry] });
    bytes += new TextEncoder().encode(entry.source + entry.target).length;
    if (bytes > MAX_GLOSSARY_BYTES) throw Error('Glossary exceeds the 4 MiB limit.');
    entries.push(entry);
  };
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else {
          quoted = false;
          closed = true;
        }
      } else field += c;
    } else if (c === delimiter) {
      if (row.length) throw Error('Glossary files must contain exactly two columns: source and target.');
      row.push(field);
      field = '';
      closed = false;
    } else if (c === '\r' || c === '\n') {
      if (c === '\r' && text[i + 1] === '\n') i++;
      row.push(field);
      field = '';
      closed = false;
      if (row.length !== 1 || row[0].trim()) finishRow();
      row = [];
    } else if (c === '"' && !field && !closed) quoted = true;
    else {
      if (closed || c === '"') throw Error('Malformed glossary file: invalid quoting.');
      field += c;
    }
  }
  if (quoted) throw Error('Malformed glossary file: unclosed quoted field.');
  if (field || row.length || closed) {
    row.push(field);
    finishRow();
  }
  // Imports preserve duplicate rows for the existing explicit merge preview.
  return entries;
}
export function encodeGlossary(entries: GlossaryEntry[], format: string): string {
  const delimiter = format === 'csv' ? ',' : format === 'tsv' ? '\t' : '';
  if (!delimiter) throw Error('Choose a CSV or TSV file.');
  entries = normalizeGlossary({ ...emptyGlossary(), entries }).entries;
  const quote = (s: string) => '"' + s.replaceAll('"', '""') + '"';
  return (
    '\uFEFFsource' +
    delimiter +
    'target\r\n' +
    entries.map((e) => quote(e.source) + delimiter + quote(e.target) + '\r\n').join('')
  );
}
