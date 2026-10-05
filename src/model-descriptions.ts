import type { CleanupSettings } from './types';
export const cleanupChoices: {
  value: CleanupSettings['method'];
  label: string;
  description: string;
  size: string;
}[] = [
  {
    value: 'solid',
    label: 'Solid fill',
    description: 'Fastest cleanup for plain balloons; no download.',
    size: 'Included',
  },
  {
    value: 'migan',
    label: 'MI-GAN',
    description: 'Small and fast; less reliable on complex artwork.',
    size: '28.18 MiB',
  },
  {
    value: 'manga_aot',
    label: 'Manga AOT',
    description: 'Compact manga cleanup model with moderate speed.',
    size: '22.00 MiB',
  },
  {
    value: 'manga_lama',
    label: 'Manga LaMa',
    description: 'Best cleanup in our tests; larger and slower.',
    size: '196.74 MiB',
  },
];
export const cleanupName = (method: CleanupSettings['method']) =>
  cleanupChoices.find((c) => c.value === method)?.label || method;
export function modelDescription(model: any): string {
  if (!model) return 'Custom model; capabilities and performance are not rated.';
  const inputs = model.modalities?.input;
  const capability = inputs?.includes('image')
    ? 'Reads dialogue images and text'
    : inputs?.includes('text')
      ? 'Translates recognized text'
      : 'Capabilities not listed';
  const price = model.cost?.input;
  return (
    capability +
    (typeof price === 'number' && Number.isFinite(price) ? ` · $${price}/1M input tokens.` : '.')
  );
}
export function ollamaDescription(
  model: { size: number; remote: boolean },
  capabilities?: string[],
) {
  if (model.remote) return 'Cloud model; unavailable in the local Ollama integration.';
  const ability = capabilities
    ? capabilities.includes('vision')
      ? 'Image and text'
      : 'Text translation'
    : 'Installed model';
  return `${ability} · ${(model.size / 2 ** 30).toFixed(1)} GiB on the selected server.`;
}
