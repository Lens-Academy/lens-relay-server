import { createContext, useContext } from 'react';
import type { ReactNode } from 'react';

export type UserRole = 'edit' | 'suggest' | 'view';

interface AuthContextValue {
  role: UserRole;
  canEdit: boolean;     // role === 'edit'
  canSuggest: boolean;  // role === 'suggest'
  canWrite: boolean;    // role === 'edit' || role === 'suggest'
}

const AuthContext = createContext<AuthContextValue | null>(null);

interface AuthProviderProps {
  role: UserRole;
  children: ReactNode;
}

export function AuthProvider({ role, children }: AuthProviderProps) {
  const value: AuthContextValue = {
    role,
    canEdit: role === 'edit',
    canSuggest: role === 'suggest',
    canWrite: role === 'edit' || role === 'suggest',
  };

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const context = useContext(AuthContext);
  if (!context) {
    // Default to edit when no AuthProvider (backwards compatibility — direct access without token)
    return { role: 'edit', canEdit: true, canSuggest: false, canWrite: true };
  }
  return context;
}
