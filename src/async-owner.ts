/** Latest-intent ownership for navigation, dialogs and page-scoped async work. */
export class AsyncOwner {
  private generation = 0;
  capture() {
    const generation = this.generation;
    return () => generation === this.generation;
  }
  begin() {
    const generation = ++this.generation;
    return () => generation === this.generation;
  }
  invalidate() {
    this.generation++;
  }
}
/** Superseded failures must not display an error on the user's newer screen. */
export async function ownedResult<T>(current: () => boolean, request: Promise<T>): Promise<T | undefined> {
  try { const value = await request; return current() ? value : undefined; }
  catch (error) { if (current()) throw error; return undefined; }
}
