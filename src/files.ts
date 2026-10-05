import { tr } from './i18n';
import { open, confirm } from '@tauri-apps/plugin-dialog';
import { native } from './bridge';
export async function chooseSources(folder = false): Promise<string[]> {
  if (!native) return ['preview://sample'];
  const result = await open({
    directory: folder,
    multiple: !folder,
    title: tr(folder ? 'Add book source folder' : 'Add images, archives or PDF'),
    filters: folder
      ? undefined
      : [{ name: tr('Manga'), extensions: ['png', 'jpg', 'jpeg', 'webp', 'cbz', 'zip', 'pdf'] }],
  });
  return result ? (typeof result === 'string' ? [result] : result) : [];
}
export async function chooseChapterImages(title: string): Promise<string[]> {
  if (!native) return ['preview://images'];
  const result = await open({
    multiple: true,
    title: tr(`Add images to ${title}`),
    filters: [{ name: tr('Images'), extensions: ['png', 'jpg', 'jpeg', 'webp'] }],
  });
  return result ? (typeof result === 'string' ? [result] : result) : [];
}
export async function chooseCover(): Promise<string | null> {
  if (!native) return null;
  const p = await open({
    filters: [{ name: tr('Cover'), extensions: ['png', 'jpg', 'jpeg', 'webp'] }],
  });
  return typeof p === 'string' ? p : null;
}
export async function confirmDelete(names: string[]): Promise<boolean> {
  const text = `Permanently delete ${names.map((n) => `“${n}”`).join(', ')}? Linked originals will remain on disk.`;
  return native ? confirm(tr(text), { title: tr('Delete'), kind: 'warning' }) : window.confirm(tr(text));
}

export async function chooseLibrary(): Promise<string | null> {
  if (!native) return 'Browser preview';
  const result = await open({ directory: true, title: tr('Choose your library folder') });
  return typeof result === 'string' ? result : null;
}
