// src/components/Layout/EditorArea.test.tsx
/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi } from 'vitest';
import { render } from '@testing-library/react';
import { EditorArea } from './EditorArea';

// Mock the providers that EditorArea needs
vi.mock('../../contexts/NavigationContext', () => ({
  useNavigation: () => ({
    metadata: null,
    folderDocs: new Map(),
    folderNames: [],
    errors: new Map(),
    onNavigate: vi.fn(),
  }),
}));

// Mock DocumentTitle to avoid metadata dependency
vi.mock('../DocumentTitle', () => ({
  DocumentTitle: () => <div data-testid="mock-document-title">Title</div>,
}));

// Mock AuthContext used by EditorArea
vi.mock('../../contexts/AuthContext', () => ({
  useAuth: () => ({ canWrite: true, role: 'editor' }),
}));

// Mock the Editor component to avoid Y.Doc complexity
vi.mock('../Editor/Editor', () => ({
  Editor: ({ onEditorReady }: { onEditorReady?: (view: unknown) => void }) => {
    // Don't call onEditorReady to keep editorView null (easier to test)
    return <div data-testid="mock-editor">Mock Editor</div>;
  },
}));

// Mock SyncStatus and PresencePanel to avoid Y.js provider dependencies
vi.mock('../SyncStatus/SyncStatus', () => ({
  SyncStatus: () => <div data-testid="mock-sync-status">Synced</div>,
}));

vi.mock('../PresencePanel/PresencePanel', () => ({
  PresencePanel: () => <div data-testid="mock-presence-panel">Presence</div>,
}));

vi.mock('../SuggestionModeToggle/SuggestionModeToggle', () => ({
  SuggestionModeToggle: () => <div data-testid="mock-suggestion-toggle">Suggestion Mode</div>,
}));

vi.mock('../SourceModeToggle/SourceModeToggle', () => ({
  SourceModeToggle: () => <div data-testid="mock-source-toggle">Source Mode</div>,
}));

vi.mock('../DebugYMapPanel', () => ({
  DebugYMapPanel: () => <div data-testid="mock-debug-panel">Debug</div>,
}));

vi.mock('../BacklinksPanel', () => ({
  BacklinksPanel: () => <div data-testid="mock-backlinks-panel" className="backlinks-panel">Backlinks</div>,
}));

vi.mock('../DiscussionPanel', () => ({
  ConnectedDiscussionPanel: () => null,
}));

describe('EditorArea', () => {
  it('renders CommentsPanel in sidebar', () => {
    const { container } = render(<EditorArea currentDocId="test-doc" />);

    const commentsPanel = container.querySelector('.comments-panel');
    expect(commentsPanel).toBeInTheDocument();
  });

  it('renders TableOfContents in sidebar', () => {
    const { container } = render(<EditorArea currentDocId="test-doc" />);

    const tocPanel = container.querySelector('.toc-panel');
    expect(tocPanel).toBeInTheDocument();
  });

  it('renders both panels in sidebar with correct layout', () => {
    const { container } = render(<EditorArea currentDocId="test-doc" />);

    expect(container.querySelector('.comments-panel')).toBeInTheDocument();
    expect(container.querySelector('.toc-panel')).toBeInTheDocument();

    // Check that sidebar uses w-64
    const sidebar = container.querySelector('aside');
    expect(sidebar).toHaveClass('w-64');
  });
});
