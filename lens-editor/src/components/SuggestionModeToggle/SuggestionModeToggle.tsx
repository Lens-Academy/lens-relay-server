import { useState, useEffect } from 'react';
import { EditorView } from '@codemirror/view';
import {
  toggleSuggestionMode,
  suggestionModeField,
} from '../Editor/extensions/criticmarkup';
import { SegmentedToggle, type SegmentedValue } from '../SegmentedToggle';
import { useAuth } from '../../contexts/AuthContext';

interface SuggestionModeToggleProps {
  view: EditorView | null;
}

/**
 * Toggle for switching between Editing and Suggesting modes.
 *
 * - For 'edit' role: Full toggle between Editing and Suggesting modes
 * - For 'suggest' role: Locked into Suggesting mode (shows badge instead of toggle)
 * - For 'view' role: Locked "Viewing" badge (no editing capabilities)
 */
export function SuggestionModeToggle({ view }: SuggestionModeToggleProps) {
  const { role, canEdit } = useAuth();
  const [isSuggestionMode, setIsSuggestionMode] = useState(false);

  // Sync local state with editor state when view changes
  useEffect(() => {
    if (!view) return;
    setIsSuggestionMode(view.state.field(suggestionModeField));
  }, [view]);

  // Force suggestion mode ON for suggest-only users
  useEffect(() => {
    if (!view || role !== 'suggest') return;
    const currentMode = view.state.field(suggestionModeField);
    if (!currentMode) {
      view.dispatch({
        effects: toggleSuggestionMode.of(true),
      });
      setIsSuggestionMode(true);
    }
  }, [view, role]);

  // View-only users: show locked badge
  if (role === 'view') {
    return (
      <span className="inline-flex items-center px-3 py-1.5 rounded-md text-sm font-medium bg-red-100 text-red-800">
        Viewing
      </span>
    );
  }

  // Suggest-only users: show locked badge
  if (role === 'suggest') {
    return (
      <span className="inline-flex items-center px-3 py-1.5 rounded-md text-sm font-medium bg-amber-100 text-amber-800">
        Suggesting
      </span>
    );
  }

  // Edit users: full toggle
  const handleChange = (value: SegmentedValue) => {
    if (!view) return;
    const newSuggestionMode = value === 'left';
    setIsSuggestionMode(newSuggestionMode);
    view.dispatch({
      effects: toggleSuggestionMode.of(newSuggestionMode),
    });
  };

  return (
    <SegmentedToggle
      leftLabel="Suggesting"
      rightLabel="Editing"
      value={isSuggestionMode ? 'left' : 'right'}
      onChange={handleChange}
      disabled={!view}
      ariaLabel="Toggle between suggesting and editing mode"
    />
  );
}
