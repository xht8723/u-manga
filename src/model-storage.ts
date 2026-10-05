import type { Settings } from './types';

export function modelDirectory(settings: Pick<Settings, 'libraryDirectory'>) {
  const path = settings.libraryDirectory;
  if (!path.trim()) return '';
  const separator = path.includes('\\') ? '\\' : '/';
  return path.replace(/[\\/]+$/, '') + separator + 'models';
}

export type ModelStorage = { library: string; directory: string };
