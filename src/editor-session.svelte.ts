import type { Page } from './types';
import type { EditorCommit } from './editor-drafts';

/** Drafts and history are owned by one visible page, never by a chapter index. */
export class EditorSession {
  pageId: string | null = null;
  generation = 0;
  epoch = 0;
  selectedRegion = $state(0);
  dirty = $state(false);
  baseline = $state<Page | null>(null);
  draft = $state<Page | null>(null);
  undo = $state<EditorCommit[]>([]);
  redo = $state<EditorCommit[]>([]);
  pendingHistory = $state<{ back: boolean; commit: EditorCommit } | null>(null);

  enter(id: string) {
    if (this.pageId === id) return;
    this.pageId = id;
    this.generation++;
    this.epoch++;
    this.selectedRegion = 0;
    this.undo = [];
    this.redo = [];
    this.baseline = null;
    this.draft = null;
    this.pendingHistory = null;
    this.dirty = false;
  }

  begin(page: Page) {
    this.enter(page.id);
    if (this.baseline) return;
    this.baseline = structuredClone(page);
    this.draft = structuredClone(page);
  }

  capture() {
    const id = this.pageId,
      generation = this.generation;
    return () => this.pageId === id && this.generation === generation;
  }
}
