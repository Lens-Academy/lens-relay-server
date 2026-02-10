const CDN_BASE = 'https://cdn.discordapp.com';

/**
 * Construct a Discord avatar URL for a user.
 *
 * - Custom avatar: CDN URL with .gif for animated (hash starts with "a_"), .png otherwise.
 * - Default avatar (null hash): uses the embed/avatars endpoint with index derived from user ID.
 */
export function getAvatarUrl(
  userId: string,
  avatarHash: string | null,
  size: number = 64,
): string {
  if (avatarHash != null) {
    const ext = avatarHash.startsWith('a_') ? 'gif' : 'png';
    return `${CDN_BASE}/avatars/${userId}/${avatarHash}.${ext}?size=${size}`;
  }

  // Default avatar: index = (userId >> 22) % 6
  const index = Number((BigInt(userId) >> 22n) % 6n);
  return `${CDN_BASE}/embed/avatars/${index}.png`;
}
