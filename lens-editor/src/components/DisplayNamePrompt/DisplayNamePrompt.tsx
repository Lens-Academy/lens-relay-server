import { useState, useEffect, useRef } from 'react';
import { useDisplayName } from '../../contexts/DisplayNameContext';

export function DisplayNamePrompt() {
  const { displayName, setDisplayName } = useDisplayName();
  const [input, setInput] = useState('');
  const [clydeError, setClydeError] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto-focus input on mount when no display name
  useEffect(() => {
    if (!displayName) {
      inputRef.current?.focus();
    }
  }, [displayName]);

  // Already have a name -- don't show
  if (displayName) return null;

  const trimmedInput = input.trim();
  const containsClyde = /clyde/i.test(trimmedInput);
  const canSubmit = trimmedInput.length > 0 && !containsClyde;

  const handleSubmit = () => {
    if (!canSubmit) return;
    setDisplayName(trimmedInput);
  };

  const handleInputChange = (value: string) => {
    setInput(value);
    // Show clyde error reactively
    setClydeError(/clyde/i.test(value.trim()));
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
        }
        if (e.key === 'Enter' && canSubmit) {
          handleSubmit();
        }
      }}
    >
      <div className="bg-white rounded-lg shadow-xl p-6 w-[400px] mx-4">
        <h2 className="text-lg font-semibold text-gray-900 mb-2">
          What should we call you?
        </h2>
        <p className="text-sm text-gray-600 mb-4">
          This name will be shown on your messages and comments.
        </p>
        <input
          ref={inputRef}
          type="text"
          value={input}
          onChange={(e) => handleInputChange(e.target.value)}
          placeholder="Enter your display name"
          className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
          maxLength={66}
        />
        {clydeError && (
          <p className="text-xs text-red-600 mt-1">
            Name cannot contain &quot;clyde&quot; (Discord restriction)
          </p>
        )}
        <button
          onClick={handleSubmit}
          disabled={!canSubmit}
          className="mt-4 w-full px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          Continue
        </button>
      </div>
    </div>
  );
}
