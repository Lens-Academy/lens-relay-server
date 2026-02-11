/** Token returned by relay server's /doc/:id/auth endpoint */
export interface ClientToken {
  url: string;
  baseUrl: string;
  docId: string;
  token?: string;
  authorization: 'full' | 'read-only';
}

/** User role for share token auth */
export type UserRole = 'edit' | 'suggest' | 'view';
