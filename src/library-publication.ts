/** Library snapshots have independent ownership from navigation and Jobs events. */
export function libraryPath(path: string) {
  let key = path.replaceAll('\\', '/');
  if (key.startsWith('//?/UNC/')) key = '//' + key.slice(8);
  else if (key.startsWith('//?/')) key = key.slice(4);
  key = key.replace(/\/+$/, '');
  return /^[a-z]:/i.test(key) || key.startsWith('//') ? key.toLowerCase() : key;
}
type Identity = { id: string; path: string };
export type LibraryTicket = { epoch: number; serial: number; id?: string; path?: string };

export class LibraryPublication<T extends Identity> {
  private root: string | null = null;
  private epoch = 0;
  private serial = 0;
  private full = 0;
  private entries = new Map<string, T>();
  private intents = new Map<string, number>();
  private removed = new Set<string>();

  activate(root: string) {
    const key = libraryPath(root);
    if (key === this.root) return false;
    this.root = key;
    this.epoch++;
    this.entries.clear();
    this.intents.clear();
    this.removed.clear();
    this.full = 0;
    return true;
  }
  beginFull(): LibraryTicket {
    this.full = ++this.serial;
    return { epoch: this.epoch, serial: this.full };
  }
  beginBook(id: string, path: string): LibraryTicket {
    const serial = ++this.serial;
    this.intents.set(id, serial);
    return { epoch: this.epoch, serial, id, path: libraryPath(path) };
  }
  changed(id: string) {
    this.intents.set(id, ++this.serial);
  }
  created(id: string) {
    this.removed.delete(id);
    this.changed(id);
  }
  remove(id: string) {
    this.changed(id);
    this.removed.add(id);
    this.entries.delete(id);
  }
  current(ticket: LibraryTicket) {
    return ticket.epoch === this.epoch && (ticket.id
      ? !this.removed.has(ticket.id) && this.intents.get(ticket.id) === ticket.serial
      : this.full === ticket.serial);
  }
  acceptBook(ticket: LibraryTicket, summary: T) {
    if (!this.current(ticket) || ticket.id !== summary.id || ticket.path !== libraryPath(summary.path)) return false;
    this.entries.set(summary.id, summary);
    return true;
  }
  acceptFull(ticket: LibraryTicket, summaries: T[]) {
    if (!this.current(ticket)) return false;
    const present = new Set(summaries.map(s => s.id));
    for (const summary of summaries) {
      if (!this.removed.has(summary.id) && (this.intents.get(summary.id) ?? 0) <= ticket.serial) {
        this.entries.set(summary.id, summary);
        this.intents.set(summary.id, ticket.serial);
      }
    }
    for (const id of this.entries.keys())
      if (!present.has(id) && (this.intents.get(id) ?? 0) <= ticket.serial) {
        this.entries.delete(id);
        this.intents.set(id, ticket.serial);
      }
    return true;
  }
  values() { return [...this.entries.values()]; }
}
