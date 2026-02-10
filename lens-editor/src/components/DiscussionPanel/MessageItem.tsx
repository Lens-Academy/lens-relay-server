import { getAvatarUrl } from '../../lib/discord-avatar';
import { formatTimestamp } from '../../lib/format-timestamp';
import type { DiscordMessage } from './useMessages';

interface MessageItemProps {
  message: DiscordMessage;
  showHeader: boolean;
}

/**
 * Single message display with avatar, display name, timestamp, and content.
 * When showHeader is false (grouped messages), only content is shown with indentation.
 */
export function MessageItem({ message, showHeader }: MessageItemProps) {
  const { author, content, timestamp } = message;
  const displayName = author.global_name ?? author.username;
  const avatarSrc = getAvatarUrl(author.id, author.avatar, 64);

  return (
    <div className="px-3 py-0.5 hover:bg-gray-50" data-testid="message-item">
      {showHeader ? (
        <div className="flex items-start gap-2 pt-2" data-testid="message-header">
          <img
            src={avatarSrc}
            alt={displayName}
            className="w-8 h-8 rounded-full flex-shrink-0 mt-0.5"
            loading="lazy"
          />
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-2">
              <span className="text-sm font-medium text-gray-900 truncate">
                {displayName}
              </span>
              {author.bot && (
                <span className="inline-flex items-center px-1 py-0.5 rounded text-[10px] font-medium leading-none bg-[#5865F2] text-white flex-shrink-0">
                  APP
                </span>
              )}
              <span className="text-xs text-gray-400 flex-shrink-0">
                {formatTimestamp(timestamp)}
              </span>
            </div>
            {content && (
              <p className="text-sm text-gray-700 whitespace-pre-wrap break-words">
                {content}
              </p>
            )}
          </div>
        </div>
      ) : (
        <div className="pl-10">
          {content && (
            <p className="text-sm text-gray-700 whitespace-pre-wrap break-words">
              {content}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
