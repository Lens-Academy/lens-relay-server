import { describe, it, expect } from 'vitest';
import { renderHook } from '@testing-library/react';
import { AuthProvider, useAuth } from './AuthContext';
import type { UserRole } from './AuthContext';
import type { ReactNode } from 'react';

function wrapper(role: UserRole) {
  return ({ children }: { children: ReactNode }) => (
    <AuthProvider role={role}>{children}</AuthProvider>
  );
}

describe('AuthContext', () => {
  it('should default to edit when no provider', () => {
    const { result } = renderHook(() => useAuth());
    expect(result.current.role).toBe('edit');
    expect(result.current.canEdit).toBe(true);
    expect(result.current.canSuggest).toBe(false);
    expect(result.current.canWrite).toBe(true);
  });

  it('should return edit role values', () => {
    const { result } = renderHook(() => useAuth(), { wrapper: wrapper('edit') });
    expect(result.current.role).toBe('edit');
    expect(result.current.canEdit).toBe(true);
    expect(result.current.canSuggest).toBe(false);
    expect(result.current.canWrite).toBe(true);
  });

  it('should return suggest role values', () => {
    const { result } = renderHook(() => useAuth(), { wrapper: wrapper('suggest') });
    expect(result.current.role).toBe('suggest');
    expect(result.current.canEdit).toBe(false);
    expect(result.current.canSuggest).toBe(true);
    expect(result.current.canWrite).toBe(true);
  });

  it('should return view role values', () => {
    const { result } = renderHook(() => useAuth(), { wrapper: wrapper('view') });
    expect(result.current.role).toBe('view');
    expect(result.current.canEdit).toBe(false);
    expect(result.current.canSuggest).toBe(false);
    expect(result.current.canWrite).toBe(false);
  });
});
