import type * as Y from 'yjs';
import { useDiscussion } from './useDiscussion';
import { useMessages } from './useMessages';
import { MessageList } from './MessageList';

interface DiscussionPanelProps {
  /** Y.Doc to read frontmatter from. Pass null when no doc is loaded. */
  doc: Y.Doc | null;
}

/**
 * Discussion panel that conditionally renders based on `discussion` frontmatter.
 * Shows Discord channel messages when a discussion URL is present.
 * Returns null when no discussion field exists.
 *
 * In tests, pass doc directly. In production, use ConnectedDiscussionPanel
 * which reads from YDocProvider context.
 */
export function DiscussionPanel({ doc }: DiscussionPanelProps) {
  const { channelId } = useDiscussion(doc);
  const { messages, channelName, loading, error, refetch } = useMessages(channelId);

  // Don't render anything if no discussion URL in frontmatter
  if (!channelId) return null;

  return (
    <aside
      className="w-80 flex-shrink-0 border-l border-gray-200 bg-white flex flex-col"
      role="complementary"
      aria-label="Discussion"
    >
      {/* Header */}
      <div className="px-3 py-2 border-b border-gray-200">
        <h3 className="text-sm font-semibold text-gray-700">
          {channelName ? `#${channelName}` : 'Discussion'}
        </h3>
      </div>

      {/* Content area */}
      {loading && messages.length === 0 ? (
        <div className="flex-1 flex items-center justify-center p-4">
          <p className="text-sm text-gray-400">Loading messages...</p>
        </div>
      ) : error ? (
        <div className="flex-1 flex flex-col items-center justify-center p-4 gap-3">
          <p className="text-sm text-red-600">{error}</p>
          <button
            onClick={refetch}
            className="px-3 py-1.5 text-sm bg-gray-100 hover:bg-gray-200 rounded transition-colors"
          >
            Retry
          </button>
        </div>
      ) : (
        <MessageList messages={messages} />
      )}
    </aside>
  );
}
