import { Tree } from 'react-arborist';
import { FileTreeNode } from './FileTreeNode';
import type { TreeNode } from '../../lib/tree-utils';

interface FileTreeProps {
  data: TreeNode[];
  onSelect?: (docId: string) => void;
  openAll?: boolean;
}

export function FileTree({ data, onSelect, openAll }: FileTreeProps) {
  return (
    <Tree<TreeNode>
      data={data}
      openByDefault={true}
      indent={16}
      rowHeight={28}
      width="100%"
      height={600}
      overscanCount={5}
      disableDrag
      disableDrop
      disableMultiSelection
      onSelect={(nodes) => {
        if (nodes.length === 1 && !nodes[0].data.isFolder && nodes[0].data.docId) {
          onSelect?.(nodes[0].data.docId);
        }
      }}
    >
      {FileTreeNode}
    </Tree>
  );
}
