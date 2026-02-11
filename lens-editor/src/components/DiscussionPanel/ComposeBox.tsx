import { useState, useCallback, type KeyboardEvent } from 'react';
import TextareaAutosize from 'react-textarea-autosize';
import { useDisplayName } from '../../contexts/DisplayNameContext';

interface ComposeBoxProps {
  channelName: string | null;
  onSend: (content: string, username: string) => Promise<void>;
  disabled?: boolean;
}

/**
 * Compose input for the discussion panel.
 * Auto-growing textarea with Enter-to-send, Shift+Enter for newlines,
 * double-send prevention, and inline error display on failure.
 */
export function ComposeBox({ channelName, onSend, disabled }: ComposeBoxProps) {
  const [value, setValue] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { displayName } = useDisplayName();

  const canSend = value.trim().length > 0 && !sending && !disabled;

  const handleSend = useCallback(async () => {
    if (!canSend) return;

    const content = value.trim();
    setValue('');
    setSending(true);
    setError(null);

    if (!displayName) {
      setValue(content);
      setError('Set a display name first');
      setSending(false);
      return;
    }

    try {
      await onSend(content, displayName);
    } catch {
      setValue(content);
      setError('Failed to send \u2014 try again');
    } finally {
      setSending(false);
    }
  }, [canSend, value, displayName, onSend]);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend]
  );

  return (
    <div className="border-t border-gray-200 px-3 py-2">
      {error && <p className="text-xs text-red-600 mb-1">{error}</p>}
      <div className="flex items-end gap-2">
        <TextareaAutosize
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
            setError(null);
          }}
          onKeyDown={handleKeyDown}
          placeholder={channelName ? `Message #${channelName}` : 'Send a message'}
          maxRows={4}
          minRows={1}
          disabled={sending || disabled}
          className="flex-1 resize-none px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
        />
        <button
          onClick={handleSend}
          disabled={!canSend}
          className="p-2 text-blue-600 hover:text-blue-700 disabled:text-gray-300 disabled:cursor-not-allowed transition-colors flex-shrink-0"
          aria-label="Send message"
        >
          <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
            <path d="M2.94 17.94a1 1 0 01-.34-1.47l4.13-6.47-4.13-6.47a1 1 0 011.34-1.47l14 7a1 1 0 010 1.88l-14 7a1 1 0 01-1-.06z" />
          </svg>
        </button>
      </div>
    </div>
  );
}
