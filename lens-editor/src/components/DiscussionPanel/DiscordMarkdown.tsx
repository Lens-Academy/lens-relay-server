import { useMemo, type ReactNode } from 'react';
import { parse, type SimpleMarkdown } from 'discord-markdown-parser';

type ASTNode = SimpleMarkdown.SingleASTNode;

/**
 * Recursively render an array of AST nodes to React elements.
 */
function renderNodes(nodes: ASTNode[]): ReactNode[] {
  return nodes.map((node, i) => renderNode(node, i));
}

/**
 * Render a single AST node to a React element.
 *
 * The parser sometimes places raw strings in content arrays,
 * so we guard against non-object nodes at the top.
 */
function renderNode(node: ASTNode | string, key: number): ReactNode {
  if (typeof node === 'string') {
    return node;
  }

  const children =
    Array.isArray(node.content) ? renderNodes(node.content) : undefined;

  switch (node.type) {
    case 'text':
      return node.content as string;

    case 'strong':
      return <strong key={key}>{children}</strong>;

    case 'em':
      return <em key={key}>{children}</em>;

    case 'underline':
      return <u key={key}>{children}</u>;

    case 'strikethrough':
      return <del key={key}>{children}</del>;

    case 'inlineCode':
      return (
        <code
          key={key}
          className="bg-gray-100 dark:bg-gray-800 px-1 py-0.5 rounded text-sm font-mono"
        >
          {node.content as string}
        </code>
      );

    case 'codeBlock':
      return (
        <pre
          key={key}
          className="bg-gray-100 dark:bg-gray-800 p-2 rounded text-sm font-mono overflow-x-auto my-1"
        >
          <code>{node.content as string}</code>
        </pre>
      );

    case 'blockQuote':
      return (
        <blockquote
          key={key}
          className="border-l-4 border-gray-300 pl-2 my-1"
        >
          {children}
        </blockquote>
      );

    case 'spoiler':
      return (
        <span
          key={key}
          className="bg-gray-700 text-gray-700 hover:bg-transparent hover:text-inherit rounded px-0.5 cursor-pointer transition-colors"
        >
          {children}
        </span>
      );

    case 'url':
    case 'autolink':
      return (
        <a
          key={key}
          href={node.target}
          target="_blank"
          rel="noopener noreferrer"
          className="text-blue-600 hover:underline"
        >
          {children ?? (node.content as string)}
        </a>
      );

    case 'br':
    case 'newline':
      return <br key={key} />;

    case 'heading':
      return (
        <strong key={key} className="block">
          {children}
        </strong>
      );

    case 'subtext':
      return (
        <small key={key} className="text-xs text-gray-500">
          {children}
        </small>
      );

    // Discord-specific nodes we don't resolve (would need API calls).
    // Show raw mention syntax as a styled badge.
    case 'user':
    case 'channel':
    case 'role':
    case 'everyone':
    case 'here':
    case 'slashCommand':
    case 'guildNavigation':
      return (
        <span
          key={key}
          className="bg-blue-100 text-blue-800 rounded px-0.5 text-sm"
        >
          {node.type === 'everyone'
            ? '@everyone'
            : node.type === 'here'
              ? '@here'
              : `@${node.id ?? node.content ?? 'unknown'}`}
        </span>
      );

    case 'emoji':
    case 'twemoji':
    case 'emoticon':
      // Custom emoji / twemoji / emoticon -- render the name or text content
      return (
        <span key={key}>
          {typeof node.content === 'string'
            ? node.content
            : node.name
              ? `:${node.name}:`
              : (node.surrogate ?? '')}
        </span>
      );

    default:
      // Graceful fallback for any unrecognized node type
      if (Array.isArray(node.content)) {
        return <span key={key}>{renderNodes(node.content)}</span>;
      }
      if (typeof node.content === 'string') {
        return node.content;
      }
      return '';
  }
}

interface DiscordMarkdownProps {
  content: string;
}

/**
 * Parse Discord-flavored markdown and render as React elements.
 *
 * Handles: bold, italic, underline, strikethrough, inline code,
 * code blocks, blockquotes, spoilers, links, and line breaks.
 *
 * Discord-specific mentions (users, roles, channels) are shown as
 * unstyled placeholders since resolving IDs requires additional API calls.
 */
export function DiscordMarkdown({ content }: DiscordMarkdownProps) {
  const rendered = useMemo(() => {
    if (!content) return null;
    const ast = parse(content, 'normal');
    return renderNodes(ast as ASTNode[]);
  }, [content]);

  return <>{rendered}</>;
}
