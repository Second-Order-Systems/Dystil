import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SignUpForm } from "@/components/auth/sign-up-form";

describe("SignUpForm", () => {
  it("reveals password and confirmation independently without submitting", () => {
    const onSubmit = vi.fn();
    render(<SignUpForm isPending={false} onSubmit={onSubmit} />);

    const password = screen.getByLabelText("Password") as HTMLInputElement;
    const confirmation = screen.getByLabelText("Confirm password") as HTMLInputElement;
    expect(password.type).toBe("password");
    expect(confirmation.type).toBe("password");

    fireEvent.click(screen.getByRole("button", { name: "Show password" }));
    expect(password.type).toBe("text");
    expect(confirmation.type).toBe("password");
    expect(onSubmit).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Show confirm password" }));
    expect(confirmation.type).toBe("text");
    expect(screen.getByRole("button", { name: "Hide password" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Hide confirm password" }),
    ).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
