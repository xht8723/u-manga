/** RFC 4122 v4 identities for UI controls and client-created records.
 * randomUUID requires a secure context; hosted LAN HTTP does not provide one.
 * getRandomValues supplies secure random bytes on those origins too.
 * Authentication/session tokens remain generated exclusively by the native server.
 */
export function randomUuid(): string {
  const crypto = globalThis.crypto;
  if (typeof crypto?.randomUUID === 'function') return crypto.randomUUID();
  if (typeof crypto?.getRandomValues !== 'function')
    throw new Error('This browser cannot generate secure identifiers. Use an up-to-date browser.');
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
