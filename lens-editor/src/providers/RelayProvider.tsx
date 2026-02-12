import { YDocProvider, useYjsProvider } from '@y-sweet/react';
import { EVENT_CONNECTION_STATUS } from '@y-sweet/client';
import { getClientToken } from '../lib/auth';
import { loadTimer } from '../lib/load-timing';
import { type ReactNode, useEffect, useRef } from 'react';

interface RelayProviderProps {
  docId: string;
  children: ReactNode;
}

/** Invisible component that listens to provider status events for timing */
function ConnectionTimingTracker() {
  const provider = useYjsProvider();

  useEffect(() => {
    const handler = (status: string) => {
      loadTimer.mark(`ws-${status}`);
    };
    provider.on(EVENT_CONNECTION_STATUS, handler);
    return () => {
      provider.off(EVENT_CONNECTION_STATUS, handler);
    };
  }, [provider]);

  return null;
}

export function RelayProvider({ docId, children }: RelayProviderProps) {
  // Mark provider-mount only once per docId (React StrictMode double-renders)
  const markedRef = useRef<string | null>(null);
  if (markedRef.current !== docId) {
    markedRef.current = docId;
    loadTimer.mark('provider-mount');
  }

  return (
    <YDocProvider
      docId={docId}
      authEndpoint={() => getClientToken(docId)}
    >
      <ConnectionTimingTracker />
      {children}
    </YDocProvider>
  );
}
