import { useEffect, useState } from "react";

export type ChatFilePreview = {
  path: string;
  visible: boolean;
  previousMode: string;
};

/**
 * File previews belong to a particular chat. They must not leak into the next
 * conversation when the user switches threads.
 */
export function useChatFilePreview(conversationId: string | null) {
  const [filePreview, setFilePreview] = useState<ChatFilePreview | null>(null);

  useEffect(() => {
    setFilePreview(null);
  }, [conversationId]);

  return {
    filePreview,
    openFilePreview: (path: string, previousMode: string) =>
      setFilePreview({ path, visible: true, previousMode }),
    closeFilePreview: () => setFilePreview(null),
  };
}
