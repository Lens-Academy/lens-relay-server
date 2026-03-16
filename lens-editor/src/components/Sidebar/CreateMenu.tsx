import { useState, useRef, useEffect, useCallback } from 'react';

interface CreateMenuProps {
  folderName: string;
  onCreateDocument?: () => void;
  onCreateFolder?: () => void;
}

export function CreateMenu({ folderName, onCreateDocument, onCreateFolder }: CreateMenuProps) {
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const handleClose = useCallback(() => setOpen(false), []);

  // Close on click outside
  useEffect(() => {
    if (!open) return;
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        handleClose();
      }
    };
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [open, handleClose]);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') handleClose();
    };
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [open, handleClose]);

  return (
    <div ref={menuRef} className="ml-auto flex-shrink-0 relative">
      <button
        aria-label={`Create in ${folderName}`}
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open);
        }}
        className="p-0.5 text-gray-400 hover:text-gray-600 hover:bg-gray-200 rounded"
      >
        <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
        </svg>
      </button>

      {open && (
        <div className="absolute right-0 top-full mt-1 bg-white rounded shadow-lg border border-gray-200 py-1 min-w-[140px] z-50">
          {onCreateDocument && (
            <button
              className="w-full text-left px-3 py-1.5 text-sm text-gray-700 hover:bg-gray-100"
              onClick={(e) => {
                e.stopPropagation();
                onCreateDocument();
                handleClose();
              }}
            >
              New File
            </button>
          )}
          {onCreateFolder && (
            <button
              className="w-full text-left px-3 py-1.5 text-sm text-gray-700 hover:bg-gray-100"
              onClick={(e) => {
                e.stopPropagation();
                onCreateFolder();
                handleClose();
              }}
            >
              New Folder
            </button>
          )}
        </div>
      )}
    </div>
  );
}
