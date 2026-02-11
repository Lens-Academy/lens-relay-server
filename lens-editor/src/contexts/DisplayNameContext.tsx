import { createContext, useContext, useState, useCallback, type ReactNode } from 'react';

const STORAGE_KEY = 'lens-editor-display-name';

interface DisplayNameContextValue {
  displayName: string | null;
  setDisplayName: (name: string) => void;
}

const DisplayNameContext = createContext<DisplayNameContextValue | null>(null);

export function DisplayNameProvider({ children }: { children: ReactNode }) {
  const [displayName, setDisplayNameState] = useState<string | null>(() => {
    try {
      return localStorage.getItem(STORAGE_KEY);
    } catch {
      return null;
    }
  });

  const setDisplayName = useCallback((name: string) => {
    const trimmed = name.trim();
    if (!trimmed) return;
    setDisplayNameState(trimmed);
    try {
      localStorage.setItem(STORAGE_KEY, trimmed);
    } catch {
      // localStorage full or unavailable -- state still works for session
    }
  }, []);

  return (
    <DisplayNameContext.Provider value={{ displayName, setDisplayName }}>
      {children}
    </DisplayNameContext.Provider>
  );
}

export function useDisplayName(): DisplayNameContextValue {
  const ctx = useContext(DisplayNameContext);
  if (!ctx) throw new Error('useDisplayName must be used within DisplayNameProvider');
  return ctx;
}
