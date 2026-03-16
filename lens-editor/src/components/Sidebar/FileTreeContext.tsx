import { createContext, useContext, type ReactNode } from 'react';

export interface FileTreeContextValue {
  onRequestRename?: (path: string) => void;
  onRequestDelete?: (path: string, name: string) => void;
  onRequestMove?: (path: string, docId: string) => void;
  onRenameSubmit?: (oldPath: string, newName: string, docId: string) => void;
  onCreateDocument?: (folderPath: string) => void;
  onCreateFolder?: (folderPath: string) => void;
  onOpenNewTab?: (docId: string) => void;
  editingPath: string | null;
  onEditingChange: (path: string | null) => void;
  activeDocId?: string;
}

const FileTreeContext = createContext<FileTreeContextValue | null>(null);

export function FileTreeProvider({
  children,
  value,
}: {
  children: ReactNode;
  value: FileTreeContextValue;
}) {
  return (
    <FileTreeContext.Provider value={value}>
      {children}
    </FileTreeContext.Provider>
  );
}

export function useFileTreeContext() {
  const context = useContext(FileTreeContext);
  if (!context) {
    throw new Error('useFileTreeContext must be used within FileTreeProvider');
  }
  return context;
}
