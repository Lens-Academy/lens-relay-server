/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, cleanup, waitFor, fireEvent } from '@testing-library/react';
import * as Y from 'yjs';
import { DiscussionPanel } from './DiscussionPanel';
import messagesFixture from './__fixtures__/discord-messages.json';
import channelFixture from './__fixtures__/discord-channel.json';

// ---- Y.Doc test helpers ----

function createTestDoc(markdownContent: string): Y.Doc {
  const doc = new Y.Doc();
  doc.getText('contents').insert(0, markdownContent);
  return doc;
}

// We pass Y.Doc directly into DiscussionPanel as a prop rather than mocking
// @y-sweet/react, since the component will accept an optional `doc` prop for
// testability (falling back to useYDoc() in production).

// ---- Fixture helpers ----

// Get messages that have actual text content (type 0 = default message)
const textMessages = messagesFixture.filter(
  (m: { type: number; content: string }) => m.type === 0 && m.content.length > 0,
);

// The fixture is newest-first; the component should reverse to chronological (oldest-first)
const chronologicalMessages = [...textMessages].reverse();

// ---- Fetch mock helpers ----

function mockFetchSuccess() {
  return vi.fn((url: string) => {
    if (url.includes('/messages')) {
      return Promise.resolve(
        new Response(JSON.stringify(messagesFixture), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );
    }
    if (url.match(/\/api\/discord\/channels\/\d+$/)) {
      return Promise.resolve(
        new Response(JSON.stringify(channelFixture), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );
    }
    return Promise.resolve(new Response('Not found', { status: 404 }));
  });
}

function mockFetchError() {
  return vi.fn(() =>
    Promise.resolve(
      new Response(JSON.stringify({ message: 'Internal Server Error' }), {
        status: 500,
      }),
    ),
  );
}

function mockFetchEmpty() {
  return vi.fn((url: string) => {
    if (url.includes('/messages')) {
      return Promise.resolve(
        new Response(JSON.stringify([]), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );
    }
    if (url.match(/\/api\/discord\/channels\/\d+$/)) {
      return Promise.resolve(
        new Response(JSON.stringify(channelFixture), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );
    }
    return Promise.resolve(new Response('Not found', { status: 404 }));
  });
}

function mockFetchLoading() {
  // Returns a fetch that never resolves (for testing loading state)
  return vi.fn(
    () =>
      new Promise<Response>(() => {
        // intentionally never resolves
      }),
  );
}

// ---- Test suites ----

const DISCUSSION_URL = 'https://discord.com/channels/1443369661847834688/1443369662560735264';

describe('DiscussionPanel - with discussion frontmatter', () => {
  let doc: Y.Doc;

  beforeEach(() => {
    doc = createTestDoc(`---\ndiscussion: ${DISCUSSION_URL}\n---\nSome document content`);
    vi.stubGlobal('fetch', mockFetchSuccess());
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders message text from fixture data', async () => {
    render(<DiscussionPanel doc={doc} />);

    // The first chronological text message content should appear
    // Use getAllByText since the fixture has duplicate message content
    const firstMsg = chronologicalMessages[0];
    await waitFor(() => {
      const matches = screen.getAllByText(firstMsg.content, { exact: false });
      expect(matches.length).toBeGreaterThan(0);
    });
  });

  it('renders usernames', async () => {
    render(<DiscussionPanel doc={doc} />);

    // lucbrinkman has global_name "Luc Brinkman" -- appears multiple times due to grouping headers
    await waitFor(() => {
      const matches = screen.getAllByText('Luc Brinkman');
      expect(matches.length).toBeGreaterThan(0);
    });
  });

  it('renders bot username when global_name is null', async () => {
    render(<DiscussionPanel doc={doc} />);

    // "Luc's Dev App" has null global_name, should fall back to username
    // Appears multiple times since the bot posts many messages
    await waitFor(() => {
      const matches = screen.getAllByText("Luc's Dev App");
      expect(matches.length).toBeGreaterThan(0);
    });
  });

  it('renders avatar images with correct src', async () => {
    render(<DiscussionPanel doc={doc} />);

    await waitFor(() => {
      // lucbrinkman has avatar hash "8268a38d449e8329c73a19a9b52a02ec"
      const avatars = screen.getAllByRole('img');
      const lucAvatar = avatars.find((img) =>
        (img as HTMLImageElement).src.includes('8268a38d449e8329c73a19a9b52a02ec'),
      );
      expect(lucAvatar).toBeDefined();
    });
  });

  it('renders default avatar for users without custom avatar', async () => {
    render(<DiscussionPanel doc={doc} />);

    await waitFor(() => {
      // "Luc's Dev App" (id: 1443370056875642980) has null avatar, should use default
      const avatars = screen.getAllByRole('img');
      const defaultAvatar = avatars.find((img) =>
        (img as HTMLImageElement).src.includes('embed/avatars/'),
      );
      expect(defaultAvatar).toBeDefined();
    });
  });

  it('renders formatted timestamps', async () => {
    render(<DiscussionPanel doc={doc} />);

    // Timestamps from fixture are from Jan/Feb 2026.
    // formatTimestamp will show either relative (e.g. "3d ago") or absolute (e.g. "Jan 15")
    await waitFor(() => {
      const timeElements = screen.getAllByText(/ago|just now|Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec/);
      expect(timeElements.length).toBeGreaterThan(0);
    });
  });

  it('renders channel name in header', async () => {
    render(<DiscussionPanel doc={doc} />);

    await waitFor(() => {
      expect(screen.getByText(`#${channelFixture.name}`)).toBeInTheDocument();
    });
  });

  it('groups consecutive messages from same author within 5 minutes', async () => {
    render(<DiscussionPanel doc={doc} />);

    await waitFor(() => {
      // The fixture has consecutive bot messages very close in time.
      // In the reversed (chronological) order, there should be grouped messages
      // where some have no visible header (avatar + username).
      // Count the data-testid="message-header" elements vs total messages.
      const headers = document.querySelectorAll('[data-testid="message-header"]');
      const items = document.querySelectorAll('[data-testid="message-item"]');
      // With grouping, headers < items (some messages are grouped)
      expect(items.length).toBeGreaterThan(0);
      expect(headers.length).toBeLessThan(items.length);
    });
  });

  it('shows loading state before messages arrive', async () => {
    vi.stubGlobal('fetch', mockFetchLoading());

    render(<DiscussionPanel doc={doc} />);

    // Should show loading indicator immediately
    expect(screen.getByText(/loading/i)).toBeInTheDocument();
  });
});

describe('DiscussionPanel - without discussion frontmatter', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders nothing when no discussion field', () => {
    const doc = createTestDoc('---\ntitle: No Discussion\n---\nJust content');
    const { container } = render(<DiscussionPanel doc={doc} />);
    expect(container.innerHTML).toBe('');
  });

  it('renders nothing when no frontmatter at all', () => {
    const doc = createTestDoc('Just plain content');
    const { container } = render(<DiscussionPanel doc={doc} />);
    expect(container.innerHTML).toBe('');
  });

  it('renders nothing when doc is null', () => {
    const { container } = render(<DiscussionPanel doc={null} />);
    expect(container.innerHTML).toBe('');
  });
});

describe('DiscussionPanel - error states', () => {
  let doc: Y.Doc;

  beforeEach(() => {
    doc = createTestDoc(`---\ndiscussion: ${DISCUSSION_URL}\n---\nContent`);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('shows error message when fetch fails', async () => {
    vi.stubGlobal('fetch', mockFetchError());

    render(<DiscussionPanel doc={doc} />);

    await waitFor(() => {
      expect(screen.getByText(/error|failed|could not/i)).toBeInTheDocument();
    });
  });

  it('shows retry button on error', async () => {
    vi.stubGlobal('fetch', mockFetchError());

    render(<DiscussionPanel doc={doc} />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
    });
  });

  it('retries fetch when retry button clicked', async () => {
    const errorFetch = mockFetchError();
    vi.stubGlobal('fetch', errorFetch);

    render(<DiscussionPanel doc={doc} />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
    });

    // Clear call count
    const callCountBefore = errorFetch.mock.calls.length;

    // Click retry
    fireEvent.click(screen.getByRole('button', { name: /retry/i }));

    // Should have made additional fetch calls
    await waitFor(() => {
      expect(errorFetch.mock.calls.length).toBeGreaterThan(callCountBefore);
    });
  });
});

describe('DiscussionPanel - empty channel', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('shows empty state message', async () => {
    const doc = createTestDoc(`---\ndiscussion: ${DISCUSSION_URL}\n---\nContent`);
    vi.stubGlobal('fetch', mockFetchEmpty());

    render(<DiscussionPanel doc={doc} />);

    await waitFor(() => {
      expect(screen.getByText(/no messages/i)).toBeInTheDocument();
    });
  });
});
