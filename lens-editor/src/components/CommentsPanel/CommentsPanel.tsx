// src/components/CommentsPanel/CommentsPanel.tsx
import { useState } from 'react';
import type { EditorView } from '@codemirror/view';
import { useComments } from './useComments';
import { AddCommentForm } from './AddCommentForm';
import { getCurrentAuthor } from '../Editor/extensions/criticmarkup';
import { formatTimestamp } from '../../lib/format-timestamp';
import type { CriticMarkupRange, CommentThread as CommentThreadType } from '../../lib/criticmarkup-parser';

interface CommentsPanelProps {
  view: EditorView | null;
  stateVersion?: number; // Triggers re-render on doc changes
}

/**
 * Scroll the editor to a specific position and focus it.
 */
function scrollToPosition(view: EditorView, pos: number): void {
  view.dispatch({
    selection: { anchor: pos },
    scrollIntoView: true,
  });
  view.focus();
}

/**
 * Insert a comment at the specified position.
 * Used for both new comments and replies (replies insert at thread end for adjacency).
 */
function insertCommentAt(view: EditorView, content: string, pos: number): void {
  const author = getCurrentAuthor();
  const timestamp = Date.now();
  const meta = JSON.stringify({ author, timestamp });
  const markup = `{>>${meta}@@${content}<<}`;

  view.dispatch({
    changes: { from: pos, insert: markup },
  });
}

/**
 * Component for displaying a single comment.
 */
function CommentItem({
  comment,
  onClick,
}: {
  comment: CriticMarkupRange;
  onClick?: () => void;
}) {
  const author = comment.metadata?.author || 'Anonymous';
  const timestamp = comment.metadata?.timestamp;

  return (
    <div
      className="comment-item px-3 py-2 cursor-pointer hover:bg-gray-50"
      onClick={onClick}
    >
      <div className="flex items-center gap-2 mb-1">
        <span className="text-sm font-medium text-gray-900">{author}</span>
        {timestamp && (
          <span className="text-xs text-gray-400">
            {formatTimestamp(timestamp)}
          </span>
        )}
      </div>
      <p className="text-sm text-gray-700">{comment.content}</p>
    </div>
  );
}

/**
 * Component for displaying a thread of comments.
 */
function CommentThread({
  thread,
  view,
}: {
  thread: CommentThreadType;
  view: EditorView;
}) {
  const [showReplyForm, setShowReplyForm] = useState(false);

  const rootComment = thread.comments[0];
  const replies = thread.comments.slice(1);
  const replyCount = replies.length;

  const handleReply = (content: string) => {
    insertCommentAt(view, content, thread.to);
    setShowReplyForm(false);
  };

  return (
    <div className="comment-thread">
      {/* Root comment */}
      <CommentItem
        comment={rootComment}
        onClick={() => scrollToPosition(view, rootComment.contentFrom)}
      />

      {/* Reply count and button */}
      <div className="px-3 py-1 flex items-center gap-2">
        {replyCount > 0 && (
          <span className="text-xs text-gray-500">
            {replyCount} {replyCount === 1 ? 'reply' : 'replies'}
          </span>
        )}
        <button
          onClick={() => setShowReplyForm(true)}
          className="text-xs text-blue-600 hover:text-blue-800"
        >
          Reply
        </button>
      </div>

      {/* Replies (indented) */}
      {replies.map((comment, index) => (
        <div key={`reply-${comment.from}-${index}`} className="pl-4">
          <CommentItem
            comment={comment}
            onClick={() => scrollToPosition(view, comment.contentFrom)}
          />
        </div>
      ))}

      {/* Reply form */}
      {showReplyForm && (
        <div className="pl-4">
          <AddCommentForm
            onSubmit={handleReply}
            onCancel={() => setShowReplyForm(false)}
            placeholder="Write a reply..."
            submitLabel="Reply"
          />
        </div>
      )}
    </div>
  );
}

export function CommentsPanel({ view, stateVersion }: CommentsPanelProps) {
  // stateVersion triggers re-render (parent increments on doc change)
  void stateVersion;

  const [showAddForm, setShowAddForm] = useState(false);
  const threads = useComments(view);

  const handleAddComment = (content: string) => {
    if (!view) return;
    const pos = view.state.selection.main.head;
    insertCommentAt(view, content, pos);
    setShowAddForm(false);
  };

  if (!view) {
    return (
      <div className="comments-panel p-3 text-sm text-gray-500">
        No document open
      </div>
    );
  }

  if (threads.length === 0) {
    return (
      <div className="comments-panel flex flex-col h-full">
        <h3 className="px-3 py-2 text-xs font-semibold text-gray-500 uppercase tracking-wider border-b border-gray-200 flex items-center justify-between">
          <span>Comments</span>
          <button
            onClick={() => setShowAddForm(true)}
            className="text-blue-600 hover:text-blue-800 normal-case font-normal"
          >
            + Add Comment
          </button>
        </h3>
        {showAddForm && (
          <AddCommentForm
            onSubmit={handleAddComment}
            onCancel={() => setShowAddForm(false)}
          />
        )}
        <div className="p-3 text-sm text-gray-500">
          No comments in document
        </div>
      </div>
    );
  }

  return (
    <div className="comments-panel flex flex-col h-full">
      <h3 className="px-3 py-2 text-xs font-semibold text-gray-500 uppercase tracking-wider border-b border-gray-200 flex items-center justify-between">
        <span>Comments</span>
        <button
          onClick={() => setShowAddForm(true)}
          className="text-blue-600 hover:text-blue-800 normal-case font-normal"
        >
          + Add Comment
        </button>
      </h3>
      {showAddForm && (
        <AddCommentForm
          onSubmit={handleAddComment}
          onCancel={() => setShowAddForm(false)}
        />
      )}
      <ul className="py-2">
        {threads.map((thread, threadIndex) => (
          <li
            key={`thread-${thread.from}-${threadIndex}`}
            className="border-b border-gray-100 last:border-0"
          >
            <CommentThread thread={thread} view={view} />
          </li>
        ))}
      </ul>
    </div>
  );
}
