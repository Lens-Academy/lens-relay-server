import { useState, useEffect } from 'react';
import type * as Y from 'yjs';
import { extractFrontmatter } from '../../lib/frontmatter';
import { parseDiscordUrl } from '../../lib/discord-url';

interface DiscussionInfo {
  channelId: string | null;
  guildId: string | null;
}

/**
 * Hook: extracts discussion channel ID from Y.Doc text.
 * Observes the Y.Text 'contents' for frontmatter changes.
 *
 * @param doc - Y.Doc to observe (null = no doc loaded)
 */
export function useDiscussion(doc: Y.Doc | null): DiscussionInfo {
  const [info, setInfo] = useState<DiscussionInfo>({ channelId: null, guildId: null });

  useEffect(() => {
    if (!doc) {
      setInfo({ channelId: null, guildId: null });
      return;
    }

    const ytext = doc.getText('contents');

    function parse() {
      const text = ytext.toString();
      const fm = extractFrontmatter(text);
      if (!fm?.discussion || typeof fm.discussion !== 'string') {
        setInfo({ channelId: null, guildId: null });
        return;
      }

      const parsed = parseDiscordUrl(fm.discussion);
      if (!parsed) {
        setInfo({ channelId: null, guildId: null });
        return;
      }

      setInfo({ channelId: parsed.channelId, guildId: parsed.guildId });
    }

    // Parse immediately
    parse();

    // Observe changes to re-parse frontmatter
    const observer = () => {
      parse();
    };
    ytext.observe(observer);

    return () => {
      ytext.unobserve(observer);
    };
  }, [doc]);

  return info;
}
