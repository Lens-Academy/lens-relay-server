interface NewMessagesBarProps {
  count: number;
  onClick: () => void;
}

export function NewMessagesBar({ count, onClick }: NewMessagesBarProps) {
  if (count === 0) return null;

  return (
    <button
      onClick={onClick}
      className="absolute bottom-2 left-1/2 -translate-x-1/2 px-3 py-1.5 bg-blue-600 text-white text-xs font-medium rounded-full shadow-lg hover:bg-blue-700 transition-colors z-10 cursor-pointer"
      data-testid="new-messages-bar"
    >
      {count === 1 ? '1 new message' : `${count} new messages`}
    </button>
  );
}
