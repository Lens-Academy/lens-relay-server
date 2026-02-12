/**
 * Unit+1 tests for SearchPanel component.
 * Tests rendering of results, click navigation, and loading/error/empty states.
 *
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { SearchPanel } from './SearchPanel';
import type { SearchResult } from '../../lib/relay-api';

// Mock App module to control RELAY_ID
vi.mock('../../App', () => ({
  RELAY_ID: 'test-relay-id',
}));

const mockResults: SearchResult[] = [
  {
    doc_id: 'uuid-1',
    title: 'Welcome Document',
    folder: 'Lens',
    snippet: 'Welcome to <mark>Lens</mark> Relay!',
    score: 2.5,
  },
  {
    doc_id: 'uuid-2',
    title: 'Getting Started',
    folder: 'Lens Edu',
    snippet: 'This guide helps you get <mark>started</mark> with the editor.',
    score: 1.8,
  },
];

describe('SearchPanel', () => {
  it('renders result titles', () => {
    render(
      <SearchPanel
        results={mockResults}
        fileNameMatches={[]}
        loading={false}
        error={null}
        query="test"
        onNavigate={vi.fn()}
      />
    );

    expect(screen.getByText('Welcome Document')).toBeInTheDocument();
    expect(screen.getByText('Getting Started')).toBeInTheDocument();
  });

  it('renders folder labels', () => {
    render(
      <SearchPanel
        results={mockResults}
        fileNameMatches={[]}
        loading={false}
        error={null}
        query="test"
        onNavigate={vi.fn()}
      />
    );

    // "Lens" also appears in the snippet <mark> tag, so query by selector
    const folderLabels = document.querySelectorAll('span.text-gray-400');
    expect(folderLabels).toHaveLength(2);
    expect(folderLabels[0].textContent).toBe('Lens');
    expect(folderLabels[1].textContent).toBe('Lens Edu');
  });

  it('renders snippet HTML with mark tags using dangerouslySetInnerHTML', () => {
    render(
      <SearchPanel
        results={mockResults}
        fileNameMatches={[]}
        loading={false}
        error={null}
        query="test"
        onNavigate={vi.fn()}
      />
    );

    // The <mark> tags should be rendered as actual HTML elements
    const marks = document.querySelectorAll('mark');
    expect(marks.length).toBeGreaterThanOrEqual(2);
    expect(marks[0].textContent).toBe('Lens');
    expect(marks[1].textContent).toBe('started');
  });

  it('calls onNavigate with compound doc ID (RELAY_ID-doc_uuid) when clicking a result', () => {
    const onNavigate = vi.fn();

    render(
      <SearchPanel
        results={mockResults}
        fileNameMatches={[]}
        loading={false}
        error={null}
        query="test"
        onNavigate={onNavigate}
      />
    );

    fireEvent.click(screen.getByText('Welcome Document'));
    expect(onNavigate).toHaveBeenCalledWith('test-relay-id-uuid-1');

    fireEvent.click(screen.getByText('Getting Started'));
    expect(onNavigate).toHaveBeenCalledWith('test-relay-id-uuid-2');
  });

  it('shows "Searching..." when loading is true', () => {
    render(
      <SearchPanel
        results={[]}
        fileNameMatches={[]}
        loading={true}
        error={null}
        query="test"
        onNavigate={vi.fn()}
      />
    );

    expect(screen.getByText('Searching...')).toBeInTheDocument();
  });

  it('shows error message when error is set', () => {
    render(
      <SearchPanel
        results={[]}
        fileNameMatches={[]}
        loading={false}
        error="Search failed: 500"
        query="test"
        onNavigate={vi.fn()}
      />
    );

    expect(screen.getByText('Search failed: 500')).toBeInTheDocument();
  });

  it('shows "No results found" when query is non-empty but results are empty', () => {
    render(
      <SearchPanel
        results={[]}
        fileNameMatches={[]}
        loading={false}
        error={null}
        query="nonexistent"
        onNavigate={vi.fn()}
      />
    );

    expect(screen.getByText('No results found')).toBeInTheDocument();
  });

  it('renders empty when query is empty and results are empty', () => {
    const { container } = render(
      <SearchPanel
        results={[]}
        fileNameMatches={[]}
        loading={false}
        error={null}
        query=""
        onNavigate={vi.fn()}
      />
    );

    // Should not show "No results found" or any result items
    expect(screen.queryByText('No results found')).not.toBeInTheDocument();
    expect(screen.queryByText('Searching...')).not.toBeInTheDocument();
    // Container should be empty or minimal
    expect(container.querySelectorAll('li').length).toBe(0);
  });

  it('does not render folder label when folder is empty string', () => {
    const resultsNoFolder: SearchResult[] = [
      { doc_id: 'uuid-3', title: 'Orphan Doc', folder: '', snippet: 'text', score: 1 },
    ];

    render(
      <SearchPanel
        results={resultsNoFolder}
        fileNameMatches={[]}
        loading={false}
        error={null}
        query="test"
        onNavigate={vi.fn()}
      />
    );

    expect(screen.getByText('Orphan Doc')).toBeInTheDocument();
    // No folder span should be rendered (ignore section headers which also use text-gray-400)
    const folderSpans = document.querySelectorAll('span.text-gray-400');
    expect(folderSpans.length).toBe(0);
  });
});
