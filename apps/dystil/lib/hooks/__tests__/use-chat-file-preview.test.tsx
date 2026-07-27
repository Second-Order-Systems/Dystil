
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useChatFilePreview } from "../use-chat-file-preview";

describe("useChatFilePreview", () => {
  it("clears transient preview state when the conversation changes", () => {
    const { result, rerender } = renderHook(
      ({ conversationId }) => useChatFilePreview(conversationId),
      { initialProps: { conversationId: "chat-a" as string | null } },
    );

    act(() => {
      result.current.openFilePreview("/tmp/alpha.md", "browser");
    });

    expect(result.current.filePreview).toEqual({
      path: "/tmp/alpha.md",
      visible: true,
      previousMode: "browser",
    });

    rerender({ conversationId: "chat-b" });

    expect(result.current.filePreview).toBeNull();
  });
});
