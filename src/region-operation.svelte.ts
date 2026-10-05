import { listen } from '@tauri-apps/api/event';
import { call, native } from './bridge';
import { uiError, rawText, type UiText } from './i18n';
import type { RegionRequest, RegionResult } from './types';
export type RegionOperation = {
  id: string;
  pageId: string;
  startedAt: number;
  action: 'read' | 'translate';
  stage: string;
  status: 'running' | 'stopping' | 'complete' | 'failed' | 'cancelled';
  elapsedMs: number;
  receivedAt: number;
  error: UiText;
};
export class RegionOperations {
  state = $state<RegionOperation | null>(null);
  private pending = $state<Promise<RegionResult | null> | null>(null);
  get busy() {
    return !!this.pending;
  }
  private stopping() {
    return this.state?.status === 'stopping';
  }
  async cancel() {
    if (!this.pending || !this.state) return;
    if (this.state.status === 'stopping') {
      await this.pending;
      return;
    }
    this.state.status = 'stopping';
    try {
      await call('region_cancel', { id: this.state.id });
    } catch (e) {
      this.state.error = uiError(e);
    }
    await this.pending;
  }
  async run(request: RegionRequest, action: 'read' | 'translate') {
    if (this.pending) return null;
    this.state = {
      id: request.id,
      pageId: request.pageId,
      startedAt: Date.now(),
      action,
      stage: 'checking',
      status: 'running',
      elapsedMs: 0,
      receivedAt: performance.now(),
      error: '',
    };
    const work = async () => {
      let dispose = () => {};
      const update = (event: { id: string; stage: string; elapsedMs: number }) => {
        if (this.state?.id !== event.id) return;
        if (this.state.status === 'stopping') {
          void call('region_cancel', { id: event.id });
          return;
        }
        this.state.stage = event.stage;
        this.state.elapsedMs = event.elapsedMs;
        this.state.receivedAt = performance.now();
      };
      try {
        if (native)
          dispose = await listen<{ id: string; stage: string; elapsedMs: number }>(
            'region-progress',
            ({ payload }) => update(payload),
          );
        else {
          const handler = (e: Event) => update((e as CustomEvent).detail);
          window.addEventListener('region-progress', handler);
          dispose = () => window.removeEventListener('region-progress', handler);
        }
        if (this.stopping()) {
          this.state!.status = 'cancelled';
          return null;
        }
        const result = await call(action === 'read' ? 'region_read' : 'region_translate', {
          request,
        });
        if (this.stopping()) {
          this.state!.status = 'cancelled';
          return null;
        }
        if (this.state) {
          this.state.status = 'complete';
          this.state.elapsedMs = result.elapsedMs;
        }
        return result;
      } catch (e) {
        if (this.state) {
          this.state.elapsedMs += Math.max(0, performance.now() - this.state.receivedAt);
          this.state.status =
            this.state.status === 'stopping' || rawText(uiError(e)) === 'Cancelled'
              ? 'cancelled'
              : 'failed';
          if (this.state.status === 'failed') this.state.error = uiError(e);
        }
        return null;
      } finally {
        dispose();
        this.pending = null;
      }
    };
    this.pending = work();
    return this.pending;
  }
}
