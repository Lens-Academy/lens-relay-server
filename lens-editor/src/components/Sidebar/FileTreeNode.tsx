import { useState, useRef, useEffect } from 'react';
import type { NodeRendererProps } from 'react-arborist';
import type { TreeNode } from '../../lib/tree-utils';
import { FileTreeContextMenu } from './FileTreeContextMenu';
import { useFileTreeContext } from './FileTreeContext';

const INDENT_SIZE = 16; // Must match the indent prop in FileTree.tsx

export function FileTreeNode({
  node,
  style,
}: NodeRendererProps<TreeNode>) {
  const isFolder = node.data.isFolder;
  const depth = node.level;
  const ctx = useFileTreeContext();

  const isEditing = ctx.editingPath === node.data.path;

  // Check if this node is the active document
  // activeDocId is compound format: RELAY_ID-docUUID, node.data.docId is just UUID
  const isActive = !isFolder && node.data.docId && ctx.activeDocId?.endsWith(node.data.docId);

  const inputRef = useRef<HTMLInputElement>(null);
  const [editValue, setEditValue] = useState(node.data.name);
  // Guard: ignore blur events until the input has been properly focused
  const blurGuardRef = useRef(false);

  // Focus input when editing starts
  useEffect(() => {
    if (isEditing && inputRef.current) {
      blurGuardRef.current = true;
      // Delay focus+select so it happens after Radix menu close settles
      setTimeout(() => {
        if (!inputRef.current) return;
        inputRef.current.focus();
        // Select name without extension for markdown files
        const name = node.data.name;
        const dotIndex = name.lastIndexOf('.');
        if (dotIndex > 0) {
          inputRef.current.setSelectionRange(0, dotIndex);
        } else {
          inputRef.current.select();
        }
        blurGuardRef.current = false;
      }, 50);
    }
  }, [isEditing, node.data.name]);

  const handleRename = () => {
    blurGuardRef.current = true; // Arm guard before state change
    ctx.onRequestRename?.(node.data.path);
    ctx.onEditingChange(node.data.path);
    setEditValue(node.data.name);
  };

  const handleDelete = () => {
    ctx.onRequestDelete?.(node.data.path, node.data.name);
  };

  const handleSubmitRename = () => {
    const trimmed = editValue.trim();
    if (trimmed && trimmed !== node.data.name) {
      ctx.onRenameSubmit?.(node.data.path, trimmed);
    }
    ctx.onEditingChange(null);
  };

  const cancelledRef = useRef(false);

  const handleBlur = () => {
    if (blurGuardRef.current) return; // Ignore blur during edit mode transition
    if (cancelledRef.current) {
      cancelledRef.current = false;
      return; // Escape was pressed — don't save
    }
    handleSubmitRename();
  };

  const handleCancelRename = () => {
    cancelledRef.current = true;
    ctx.onEditingChange(null);
    setEditValue(node.data.name);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Stop all keys from bubbling to react-arborist tree navigation
    e.stopPropagation();
    if (e.key === 'Enter') {
      e.preventDefault();
      handleSubmitRename();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      handleCancelRename();
    }
  };

  // Build indentation guides - vertical lines for each ancestor level
  const indentGuides = [];
  for (let i = 0; i < depth; i++) {
    indentGuides.push(
      <span
        key={i}
        className="flex-shrink-0 relative"
        style={{ width: INDENT_SIZE }}
      >
        {/* Vertical guide line */}
        <span
          className="absolute left-[7px] top-0 bottom-0 w-px bg-gray-200"
          style={{ height: '100%' }}
        />
      </span>
    );
  }

  const content = (
    <div
      style={{ ...style, paddingLeft: 0 }} // Override react-arborist's padding, we handle it ourselves
      className={`flex items-center py-0.5 pr-2 cursor-pointer select-none
                  ${isActive ? 'bg-blue-100' : 'hover:bg-gray-100'}`}
      onClick={(e) => {
        if (isEditing) return;
        if (isFolder) {
          node.toggle();
        } else {
          node.select();
        }
        e.stopPropagation();
      }}
      onDoubleClick={(e) => {
        if (!isFolder && !isEditing) {
          e.stopPropagation();
          handleRename();
        }
      }}
    >
      {/* Indentation guides (vertical lines) */}
      {indentGuides}

      {/* Chevron for folders */}
      {isFolder ? (
        <svg
          className={`w-4 h-4 text-gray-500 transition-transform flex-shrink-0 ml-1
                      ${node.isOpen ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 5l7 7-7 7"
          />
        </svg>
      ) : (
        /* Spacer for files to align text with folder text (past the chevron) */
        <span className="w-5 flex-shrink-0" />
      )}

      {/* Folder icon */}
      {isFolder && (
        <svg
          className="w-4 h-4 text-gray-500 flex-shrink-0 ml-0.5"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"
          />
        </svg>
      )}

      {/* Name or edit input */}
      {isEditing ? (
        <input
          ref={inputRef}
          type="text"
          value={editValue}
          onChange={(e) => setEditValue(e.target.value)}
          onBlur={handleBlur}
          onKeyDown={handleKeyDown}
          className="flex-1 text-sm text-gray-700 bg-white border border-blue-400 rounded px-1 py-0 outline-none ml-1"
          onClick={(e) => e.stopPropagation()}
        />
      ) : (
        <span className="truncate text-sm text-gray-700 ml-1" title={node.data.path}>
          {node.data.name}
        </span>
      )}
    </div>
  );

  // Wrap with context menu if callbacks are provided
  if (ctx.onRequestDelete || ctx.onRequestRename) {
    return (
      <FileTreeContextMenu
        onRename={handleRename}
        onDelete={handleDelete}
        isFolder={isFolder}
      >
        {content}
      </FileTreeContextMenu>
    );
  }

  return content;
}
