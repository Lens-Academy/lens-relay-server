import { useYDoc } from '@y-sweet/react';
import { DiscussionPanel } from './DiscussionPanel';

/**
 * Connected wrapper that reads Y.Doc from YDocProvider context.
 * Use this in the application layout (inside RelayProvider).
 * For testing, use DiscussionPanel directly with a doc prop.
 */
export function ConnectedDiscussionPanel() {
  const doc = useYDoc();
  return <DiscussionPanel doc={doc} />;
}
