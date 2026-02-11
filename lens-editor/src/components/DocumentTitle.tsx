import { useState, useEffect, useRef, useCallback } from 'react';
import { useNavigation } from '../contexts/NavigationContext';
import { findPathByUuid } from '../lib/uuid-to-path';
import { getFolderDocForPath, getOriginalPath, getFolderNameFromPath } from '../lib/multi-folder-utils';
import { renameDocument } from '../lib/relay-api';
import { RELAY_ID } from '../App';

interface DocumentTitleProps {
  currentDocId: string;
}

export function DocumentTitle({ currentDocId }: DocumentTitleProps) {
  const { metadata, folderDocs, folderNames } = useNavigation();
  const inputRef = useRef<HTMLInputElement>(null);
  const cancelledRef = useRef(false);

  // Extract UUID from compound doc ID (RELAY_ID-UUID)
  const uuid = currentDocId.slice(RELAY_ID.length + 1);

  // Find the prefixed path for this UUID in merged metadata
  const path = findPathByUuid(uuid, metadata);

  // Extract display name (filename without .md extension)
  const displayName = path
    ? path.split('/').pop()?.replace(/\.md$/, '') ?? ''
    : '';

  const [value, setValue] = useState(displayName);

  // Update value when the document name changes externally (e.g., renamed from sidebar)
  useEffect(() => {
    setValue(displayName);
  }, [displayName]);

  const handleSubmit = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed || trimmed === displayName || !path) return;

    const doc = getFolderDocForPath(path, folderDocs, folderNames);
    if (!doc) return;
    const folderName = getFolderNameFromPath(path, folderNames)!;
    const originalPath = getOriginalPath(path, folderName);
    const parts = originalPath.split('/');
    const filename = trimmed.endsWith('.md') ? trimmed : `${trimmed}.md`;
    parts[parts.length - 1] = filename;
    const newPath = parts.join('/');
    renameDocument(doc, originalPath, newPath);
  }, [value, displayName, path, folderDocs, folderNames]);

  const handleBlur = () => {
    if (cancelledRef.current) {
      cancelledRef.current = false;
      return;
    }
    handleSubmit();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      // Let blur handle the submit to avoid double-firing
      inputRef.current?.blur();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelledRef.current = true;
      setValue(displayName);
      inputRef.current?.blur();
    }
  };

  if (!path) return null;

  return (
    <input
      ref={inputRef}
      type="text"
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onBlur={handleBlur}
      onKeyDown={handleKeyDown}
      className="w-full text-3xl font-bold text-gray-900 bg-transparent border-none outline-none
                 placeholder-gray-400 caret-gray-900"
      placeholder="Untitled"
      spellCheck={false}
    />
  );
}
