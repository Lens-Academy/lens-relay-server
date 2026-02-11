/**
 * Generate a signed share link for the Lens Editor.
 *
 * Usage:
 *   npx tsx scripts/generate-share-link.ts --role edit --folder fbd5eb54-73cc-41b0-ac28-2b93d3b4244e --expires 7d
 *   npx tsx scripts/generate-share-link.ts --role suggest --folder fbd5eb54-73cc-41b0-ac28-2b93d3b4244e --expires 24h
 *   npx tsx scripts/generate-share-link.ts --role view --folder fbd5eb54-73cc-41b0-ac28-2b93d3b4244e
 */
import { signShareToken } from '../server/share-token.ts';
import type { ShareTokenPayload } from '../server/share-token.ts';
import type { UserRole } from '../shared/types.ts';

function parseExpiry(expiresStr: string): number {
  const now = Math.floor(Date.now() / 1000);
  const match = expiresStr.match(/^(\d+)(h|d|w)$/);
  if (!match) {
    throw new Error(`Invalid expiry format: "${expiresStr}". Use e.g. "24h", "7d", "2w"`);
  }
  const [, num, unit] = match;
  const seconds = { h: 3600, d: 86400, w: 604800 }[unit]!;
  return now + parseInt(num, 10) * seconds;
}

function printUsage() {
  console.log(`Usage: npx tsx scripts/generate-share-link.ts [options]

Options:
  --role <edit|suggest|view>  Access level (required)
  --folder <id>               Folder ID (required)
  --expires <duration>         Token lifetime: e.g. "24h", "7d", "2w" (default: "7d")
  --base-url <url>            Base URL for the editor (default: http://localhost:5173)

Examples:
  npx tsx scripts/generate-share-link.ts --role edit --folder fbd5eb54-73cc-41b0-ac28-2b93d3b4244e
  npx tsx scripts/generate-share-link.ts --role suggest --folder fbd5eb54-73cc-41b0-ac28-2b93d3b4244e --expires 24h
  npx tsx scripts/generate-share-link.ts --role view --folder fbd5eb54-73cc-41b0-ac28-2b93d3b4244e --base-url https://editor.example.com`);
}

// Parse CLI args
const args = process.argv.slice(2);

if (args.includes('--help') || args.includes('-h')) {
  printUsage();
  process.exit(0);
}

function getArg(name: string): string | undefined {
  const idx = args.indexOf(name);
  return idx !== -1 && idx + 1 < args.length ? args[idx + 1] : undefined;
}

const role = getArg('--role') as UserRole | undefined;
const folder = getArg('--folder');
const expires = getArg('--expires') || '7d';
const baseUrl = getArg('--base-url') || 'http://localhost:5173';

if (!role || !['edit', 'suggest', 'view'].includes(role)) {
  console.error('Error: --role is required and must be one of: edit, suggest, view');
  printUsage();
  process.exit(1);
}

if (!folder) {
  console.error('Error: --folder is required');
  printUsage();
  process.exit(1);
}

const payload: ShareTokenPayload = {
  role,
  folder,
  expiry: parseExpiry(expires),
};

const token = signShareToken(payload);
const url = `${baseUrl}/?t=${token}`;

console.log(`\nShare Link Generated`);
console.log(`${'─'.repeat(50)}`);
console.log(`Role:    ${role}`);
console.log(`Folder:  ${folder}`);
console.log(`Expires: ${new Date(payload.expiry * 1000).toISOString()}`);
console.log(`Token:   ${token} (${token.length} chars)`);
console.log(`\nURL:\n${url}\n`);
